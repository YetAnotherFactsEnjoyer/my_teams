use std::{
    collections::HashMap,
    io::{ErrorKind, Read, Write},
    net::{SocketAddr, TcpListener},
    sync::atomic::Ordering,
    time::UNIX_EPOCH,
};

use my_teams::ffi;

use crate::{
    client::{Client, UseContext},
    models::{generate_uuid, Channel, Database, Message, Team, Thread, User},
};

const MAX_NAME_LENGTH: usize = 32;
const MAX_DESCRIPTION_LENGTH: usize = 255;
const MAX_BODY_LENGTH: usize = 512;

pub struct Server {
    listener: TcpListener,
    clients: HashMap<SocketAddr, Client>,
    db: Database,
}

impl Server {
    pub fn new(port: u16) -> std::io::Result<Self> {
        let address = format!("0.0.0.0:{port}");
        let listener = TcpListener::bind(&address)?;
        listener.set_nonblocking(true)?;

        let mut db = Database::default();
        db.load_from_file("myteams.data").ok();

        Ok(Server {
            listener,
            clients: HashMap::new(),
            db,
        })
    }

    fn accept_new_clients(&mut self) {
        match self.listener.accept() {
            Ok((stream, addr)) => {
                if stream.set_nonblocking(true).is_ok() {
                    self.clients.insert(addr, Client::new(stream));
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => println!("Error accepting new client: {e}"),
        }
    }

    fn handle_command(&mut self, addr: SocketAddr, command_line: &str) {
        let args = Self::parse_command_args(command_line);
        if args.is_empty() {
            return;
        }

        let command = args[0].as_str();

        match command {
            "/login" => self.cmd_login(addr, &args),
            "/logout" => self.cmd_logout(addr),
            "/send" => self.cmd_send(addr, &args),
            "/use" => self.cmd_use(addr, &args),
            "/users" => self.cmd_users(addr),
            "/user" => self.cmd_user(addr, &args),
            "/messages" => self.cmd_messages(addr, &args),
            "/subscribe" => self.cmd_subscribe(addr, &args),
            "/unsubscribe" => self.cmd_unsubscribe(addr, &args),
            "/create" => self.cmd_create(addr, &args),
            "/list" => self.cmd_list(addr),
            "/info" => self.cmd_info(addr),
            "/subscribed" => self.cmd_subscribed(addr, &args),
            _ => {
                if let Some(client) = self.clients.get_mut(&addr) {
                    client.queue_message("400 Unknown Command");
                }
            }
        }
    }

    fn process_clients(&mut self) {
        let mut disconnected = Vec::new();
        let mut commands_to_process = Vec::new();

        for (addr, client) in self.clients.iter_mut() {
            let mut buffer = [0; 2048];
            match client.stream.read(&mut buffer) {
                Ok(0) => disconnected.push(*addr),
                Ok(n) => {
                    client.read_buffer.extend_from_slice(&buffer[..n]);
                    while let Some(cmd) = client.extract_command() {
                        commands_to_process.push((*addr, cmd));
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(_) => disconnected.push(*addr),
            }
        }

        for (addr, cmd) in commands_to_process {
            self.handle_command(addr, &cmd);
        }

        for (addr, client) in self.clients.iter_mut() {
            if !client.write_buffer.is_empty() {
                match client.stream.write(&client.write_buffer) {
                    Ok(n) => {
                        client.write_buffer.drain(..n);
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                    Err(_) => disconnected.push(*addr),
                }
            }
        }

        for addr in disconnected {
            if let Some(client) = self.clients.remove(&addr) {
                if let Some(uuid) = client.uuid {
                    ffi::call_user_logged_out(&uuid);
                }
                let _ = client.stream.shutdown(std::net::Shutdown::Both);
            }
        }
    }

    pub fn run(&mut self) {
        println!("Server listening...");

        while my_teams::ffi::RUNNING.load(Ordering::SeqCst) {
            self.accept_new_clients();
            self.process_clients();

            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        println!("\nShutting down server. Saving data...");
        if let Err(e) = self.db.save_to_file("myteams.data") {
            println!("Error saving data: {e}");
        }
    }

    fn parse_command_args(command_line: &str) -> Vec<String> {
        let mut args = Vec::new();
        let mut current_arg = String::new();
        let mut in_quotes = false;

        for c in command_line.chars() {
            match c {
                '"' => in_quotes = !in_quotes,
                c if c.is_whitespace() && !in_quotes => {
                    if !current_arg.is_empty() {
                        args.push(current_arg.clone());
                        current_arg.clear();
                    }
                }
                _ => current_arg.push(c),
            }
        }
        if !current_arg.is_empty() {
            args.push(current_arg);
        }
        args
    }

    fn send_to(&mut self, addr: SocketAddr, msg: &str) {
        if let Some(client) = self.clients.get_mut(&addr) {
            client.queue_message(msg);
        }
    }

    fn get_client_uuid(&self, addr: SocketAddr) -> Option<String> {
        self.clients.get(&addr)?.uuid.clone()
    }

    fn is_subscribed_to_team(&self, user_uuid: &str, team_uuid: &str) -> bool {
        self.db
            .teams
            .get(team_uuid)
            .map(|team| team.subscribers.iter().any(|u| u == user_uuid))
            .unwrap_or(false)
    }

    fn context_team_uuid(context: &UseContext) -> Option<&str> {
        match context {
            UseContext::Global => None,
            UseContext::Team(team_uuid) => Some(team_uuid.as_str()),
            UseContext::Channel(team_uuid, _) => Some(team_uuid.as_str()),
            UseContext::Thread(team_uuid, _, _) => Some(team_uuid.as_str()),
        }
    }

    fn reset_client_context_if_inside_team(&mut self, addr: SocketAddr, team_uuid: &str) {
        if let Some(client) = self.clients.get_mut(&addr) {
            let must_reset = match &client.use_context {
                UseContext::Global => false,
                UseContext::Team(current_team) => current_team == team_uuid,
                UseContext::Channel(current_team, _) => current_team == team_uuid,
                UseContext::Thread(current_team, _, _) => current_team == team_uuid,
            };

            if must_reset {
                client.use_context = UseContext::Global;
            }
        }
    }

    fn send_event_to_user(&mut self, user_uuid: &str, msg: &str) {
        for client in self.clients.values_mut() {
            match client.uuid {
                Some(ref uuid) if uuid == user_uuid => {
                    client.queue_message(msg);
                    break;
                }
                _ => continue,
            }
        }
    }

    fn cmd_login(&mut self, addr: SocketAddr, args: &[String]) {
        if args.len() != 2 {
            self.send_to(addr, "400 Bad Request: Missing user_name");
            return;
        }

        let user_name = &args[1];
        if user_name.len() > MAX_NAME_LENGTH {
            self.send_to(addr, "400 Bad Request: Name too long");
            return;
        }

        let existing_uuid = self
            .db
            .users
            .values()
            .find(|u| u.name == *user_name)
            .map(|u| u.uuid.clone());

        let user_uuid = match existing_uuid {
            Some(uuid) => {
                if let Some(user) = self.db.users.get_mut(&uuid) {
                    user.is_connected = true;
                }
                uuid
            }
            None => {
                let uuid = generate_uuid();
                let new_user = User {
                    uuid: uuid.clone(),
                    name: user_name.clone(),
                    is_connected: true,
                };
                self.db.users.insert(uuid.clone(), new_user);

                ffi::call_user_created(&uuid, user_name);
                uuid
            }
        };

        if let Some(client) = self.clients.get_mut(&addr) {
            client.uuid = Some(user_uuid.clone());
            client.queue_message(&format!("200 Login OK|{user_uuid}|{user_name}"));
        }

        ffi::call_user_logged_in(&user_uuid);
    }

    fn cmd_logout(&mut self, addr: SocketAddr) {
        if let Some(client) = self.clients.get_mut(&addr) {
            if let Some(uuid) = &client.uuid {
                ffi::call_user_logged_out(uuid);

                if let Some(user) = self.db.users.get_mut(uuid) {
                    user.is_connected = false;
                }
            }
            client.uuid = None;
            client.use_context = UseContext::Global;
            client.queue_message("200 Logout OK");
        }
    }

    fn cmd_send(&mut self, addr: SocketAddr, args: &[String]) {
        if args.len() != 3 {
            self.send_to(addr, "400 Bad Request: /send \"user_uuid\" \"message\"");
            return;
        }

        let sender_uuid = match self.get_client_uuid(addr) {
            Some(uuid) => uuid,
            None => {
                self.send_to(addr, "401 Unauthorized: Please login first");
                return;
            }
        };

        let target_uuid = &args[1];
        let message_body = &args[2];

        if message_body.len() > MAX_BODY_LENGTH {
            self.send_to(addr, "400 Bad Request: Message too long");
            return;
        }

        if !self.db.users.contains_key(target_uuid) {
            self.send_to(addr, "404 Not Found: User does not exist");
            return;
        }

        let msg = Message {
            sender_uuid: sender_uuid.clone(),
            receiver_uuid: target_uuid.clone(),
            body: message_body.clone(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };
        self.db.private_messages.push(msg);

        ffi::call_private_message_sended(&sender_uuid, target_uuid, message_body);

        self.send_event_to_user(
            target_uuid,
            &format!("EVENT PM_RECEIVED|{sender_uuid}|{message_body}"),
        );

        self.send_to(addr, "200 Message Sent");
    }

    fn cmd_use(&mut self, addr: SocketAddr, args: &[String]) {
        let client_uuid = match self.get_client_uuid(addr) {
            Some(uuid) => uuid,
            None => {
                self.send_to(addr, "401 Unauthorized: Please login first");
                return;
            }
        };

        match args.len() {
            1 => {
                if let Some(client) = self.clients.get_mut(&addr) {
                    client.use_context = UseContext::Global;
                    client.queue_message("200 Context Updated");
                }
            }
            2 => {
                let team_uuid = &args[1];
                let team = match self.db.teams.get(team_uuid) {
                    Some(team) => team,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Team");
                        return;
                    }
                };

                if !team.subscribers.iter().any(|u| u == &client_uuid) {
                    self.send_to(addr, "401 Unauthorized: Not subscribed to team");
                    return;
                }

                if let Some(client) = self.clients.get_mut(&addr) {
                    client.use_context = UseContext::Team(team_uuid.clone());
                    client.queue_message("200 Context Updated");
                }
            }
            3 => {
                let team_uuid = &args[1];
                let channel_uuid = &args[2];

                let team = match self.db.teams.get(team_uuid) {
                    Some(team) => team,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Team");
                        return;
                    }
                };

                if !team.subscribers.iter().any(|u| u == &client_uuid) {
                    self.send_to(addr, "401 Unauthorized: Not subscribed to team");
                    return;
                }

                if !team.channels.contains_key(channel_uuid) {
                    self.send_to(addr, "404 Not Found: Unknown Channel");
                    return;
                }

                if let Some(client) = self.clients.get_mut(&addr) {
                    client.use_context = UseContext::Channel(team_uuid.clone(), channel_uuid.clone());
                    client.queue_message("200 Context Updated");
                }
            }
            4 => {
                let team_uuid = &args[1];
                let channel_uuid = &args[2];
                let thread_uuid = &args[3];

                let team = match self.db.teams.get(team_uuid) {
                    Some(team) => team,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Team");
                        return;
                    }
                };

                if !team.subscribers.iter().any(|u| u == &client_uuid) {
                    self.send_to(addr, "401 Unauthorized: Not subscribed to team");
                    return;
                }

                let channel = match team.channels.get(channel_uuid) {
                    Some(channel) => channel,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Channel");
                        return;
                    }
                };

                if !channel.threads.contains_key(thread_uuid) {
                    self.send_to(addr, "404 Not Found: Unknown Thread");
                    return;
                }

                if let Some(client) = self.clients.get_mut(&addr) {
                    client.use_context = UseContext::Thread(
                        team_uuid.clone(),
                        channel_uuid.clone(),
                        thread_uuid.clone(),
                    );
                    client.queue_message("200 Context Updated");
                }
            }
            _ => self.send_to(addr, "400 Bad Request: Invalid /use arguments"),
        }
    }

    fn cmd_users(&mut self, addr: SocketAddr) {
        if self.get_client_uuid(addr).is_none() {
            self.send_to(addr, "401 Unauthorized: Please login first");
            return;
        }

        let mut response = String::from("200 USERS");
        for user in self.db.users.values() {
            let status = if user.is_connected { "1" } else { "0" };
            response.push_str(&format!("|{}:{}:{status}", user.uuid, user.name));
        }
        self.send_to(addr, &response);
    }

    fn cmd_user(&mut self, addr: SocketAddr, args: &[String]) {
        if args.len() != 2 {
            self.send_to(addr, "400 Bad Request");
            return;
        }
        if self.get_client_uuid(addr).is_none() {
            self.send_to(addr, "401 Unauthorized");
            return;
        }

        let target_uuid = &args[1];
        match self.db.users.get(target_uuid) {
            Some(user) => {
                let status = if user.is_connected { "1" } else { "0" };
                self.send_to(
                    addr,
                    &format!("200 USER|{}|{}|{status}", user.uuid, user.name),
                );
            }
            None => self.send_to(addr, "404 Not Found: User not found"),
        }
    }

    fn cmd_messages(&mut self, addr: SocketAddr, args: &[String]) {
        if args.len() != 2 {
            self.send_to(addr, "400 Bad Request");
            return;
        }
        let client_uuid = match self.get_client_uuid(addr) {
            Some(uuid) => uuid,
            None => {
                self.send_to(addr, "401 Unauthorized");
                return;
            }
        };

        let target_uuid = &args[1];
        if !self.db.users.contains_key(target_uuid) {
            self.send_to(addr, "404 Not Found: User not found");
            return;
        }

        let mut response = String::from("200 MESSAGES");
        for msg in &self.db.private_messages {
            if (msg.sender_uuid == client_uuid && msg.receiver_uuid == *target_uuid)
                || (msg.sender_uuid == *target_uuid && msg.receiver_uuid == client_uuid)
            {
                response.push_str(&format!(
                    "|{}:{}:{}",
                    msg.sender_uuid, msg.timestamp, msg.body
                ));
            }
        }
        self.send_to(addr, &response);
    }

    fn cmd_subscribe(&mut self, addr: SocketAddr, args: &[String]) {
        if args.len() != 2 {
            self.send_to(addr, "400 Bad Request");
            return;
        }
        let client_uuid = match self.get_client_uuid(addr) {
            Some(uuid) => uuid,
            None => {
                self.send_to(addr, "401 Unauthorized");
                return;
            }
        };

        let team_uuid = &args[1];
        if let Some(team) = self.db.teams.get_mut(team_uuid) {
            if !team.subscribers.iter().any(|u| u == &client_uuid) {
                team.subscribers.push(client_uuid.clone());
            }
            ffi::call_user_subscribed(team_uuid, &client_uuid);
            self.send_to(addr, &format!("200 SUBSCRIBED|{client_uuid}|{team_uuid}"));
        } else {
            self.send_to(addr, "404 Not Found: Team not found");
        }
    }

    fn cmd_unsubscribe(&mut self, addr: SocketAddr, args: &[String]) {
        if args.len() != 2 {
            self.send_to(addr, "400 Bad Request");
            return;
        }

        let client_uuid = match self.get_client_uuid(addr) {
            Some(uuid) => uuid,
            None => {
                self.send_to(addr, "401 Unauthorized");
                return;
            }
        };

        let team_uuid = &args[1];
        if let Some(team) = self.db.teams.get_mut(team_uuid) {
            if !team.subscribers.iter().any(|u| u == &client_uuid) {
                self.send_to(addr, "401 Unauthorized: Not subscribed to team");
                return;
            }

            team.subscribers.retain(|u| u != &client_uuid);
            self.reset_client_context_if_inside_team(addr, team_uuid);

            ffi::call_user_unsubscribed(team_uuid, &client_uuid);
            self.send_to(addr, &format!("200 UNSUBSCRIBED|{client_uuid}|{team_uuid}"));
        } else {
            self.send_to(addr, "404 Not Found: Team not found");
        }
    }

    fn cmd_create(&mut self, addr: SocketAddr, args: &[String]) {
        let client = match self.clients.get(&addr) {
            Some(c) => c,
            None => return,
        };

        let client_uuid = match &client.uuid {
            Some(uuid) => uuid.clone(),
            None => {
                self.send_to(addr, "401 Unauthorized");
                return;
            }
        };

        let context = client.use_context.clone();

        if let Some(team_uuid) = Self::context_team_uuid(&context) {
            if !self.is_subscribed_to_team(&client_uuid, team_uuid) {
                self.send_to(addr, "401 Unauthorized: Not subscribed to team");
                return;
            }
        }

        match context {
            UseContext::Global => {
                if args.len() != 3 {
                    self.send_to(addr, "400 Bad Request");
                    return;
                }

                let name = &args[1];
                let desc = &args[2];

                if name.len() > MAX_NAME_LENGTH || desc.len() > MAX_DESCRIPTION_LENGTH {
                    self.send_to(addr, "400 Bad Request: Length error");
                    return;
                }

                if self.db.teams.values().any(|t| t.name == *name) {
                    self.send_to(addr, "409 Conflict: Team already exists");
                    return;
                }

                let new_uuid = generate_uuid();
                let team = Team {
                    uuid: new_uuid.clone(),
                    name: name.clone(),
                    description: desc.clone(),
                    subscribers: vec![client_uuid.clone()],
                    channels: HashMap::new(),
                };
                self.db.teams.insert(new_uuid.clone(), team);

                ffi::call_team_created(&new_uuid, name, &client_uuid);
                self.send_to(addr, &format!("200 TEAM_CREATED|{new_uuid}|{name}|{desc}"));
            }
            UseContext::Team(team_uuid) => {
                if args.len() != 3 {
                    self.send_to(addr, "400 Bad Request");
                    return;
                }

                let name = &args[1];
                let desc = &args[2];

                if name.len() > MAX_NAME_LENGTH || desc.len() > MAX_DESCRIPTION_LENGTH {
                    self.send_to(addr, "400 Bad Request: Length error");
                    return;
                }

                let team = match self.db.teams.get_mut(&team_uuid) {
                    Some(t) => t,
                    None => {
                        self.send_to(addr, "404 Not Found: Team unknown");
                        return;
                    }
                };

                if team.channels.values().any(|c| c.name == *name) {
                    self.send_to(addr, "409 Conflict: Channel already exists");
                    return;
                }

                let new_uuid = generate_uuid();
                let channel = Channel {
                    uuid: new_uuid.clone(),
                    name: name.clone(),
                    description: desc.clone(),
                    threads: HashMap::new(),
                };
                team.channels.insert(new_uuid.clone(), channel);

                ffi::call_channel_created(&team_uuid, &new_uuid, name);
                self.send_to(
                    addr,
                    &format!("200 CHANNEL_CREATED|{new_uuid}|{name}|{desc}"),
                );
            }
            UseContext::Channel(team_uuid, channel_uuid) => {
                if args.len() != 3 {
                    self.send_to(addr, "400 Bad Request");
                    return;
                }

                let title = &args[1];
                let body = &args[2];

                if title.len() > MAX_NAME_LENGTH || body.len() > MAX_BODY_LENGTH {
                    self.send_to(addr, "400 Bad Request: Length error");
                    return;
                }

                let team = match self.db.teams.get_mut(&team_uuid) {
                    Some(t) => t,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Team");
                        return;
                    }
                };

                let channel = match team.channels.get_mut(&channel_uuid) {
                    Some(c) => c,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Channel");
                        return;
                    }
                };

                let new_uuid = generate_uuid();
                let timestamp = std::time::SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let thread = Thread {
                    uuid: new_uuid.clone(),
                    title: title.clone(),
                    message: body.clone(),
                    author_uuid: client_uuid.clone(),
                    replies: Vec::new(),
                    timestamp,
                };
                channel.threads.insert(new_uuid.clone(), thread);

                ffi::call_thread_created(&channel_uuid, &new_uuid, &client_uuid, title, body);
                self.send_to(
                    addr,
                    &format!(
                        "200 THREAD_CREATED|{new_uuid}|{client_uuid}|{timestamp}|{title}|{body}"
                    ),
                );
            }
            UseContext::Thread(team_uuid, channel_uuid, thread_uuid) => {
                if args.len() != 2 {
                    self.send_to(addr, "400 Bad Request");
                    return;
                }

                let reply = &args[1];

                if reply.len() > MAX_BODY_LENGTH {
                    self.send_to(addr, "400 Bad Request: Reply too long");
                    return;
                }

                let team = match self.db.teams.get_mut(&team_uuid) {
                    Some(t) => t,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Team");
                        return;
                    }
                };

                let channel = match team.channels.get_mut(&channel_uuid) {
                    Some(c) => c,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Channel");
                        return;
                    }
                };

                let thread = match channel.threads.get_mut(&thread_uuid) {
                    Some(t) => t,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Thread");
                        return;
                    }
                };

                let timestamp = std::time::SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();

                let message = Message {
                    sender_uuid: client_uuid.clone(),
                    receiver_uuid: thread_uuid.clone(),
                    body: reply.clone(),
                    timestamp,
                };
                thread.replies.push(message);

                ffi::call_reply_created(&thread_uuid, &client_uuid, reply);
                self.send_to(
                    addr,
                    &format!("200 REPLY_CREATED|{thread_uuid}|{client_uuid}|{timestamp}|{reply}"),
                );
            }
        }
    }

    fn cmd_list(&mut self, addr: SocketAddr) {
        let client = match self.clients.get(&addr) {
            Some(c) => c,
            None => return,
        };

        let client_uuid = match &client.uuid {
            Some(uuid) => uuid.clone(),
            None => {
                self.send_to(addr, "401 Unauthorized: Please login first");
                return;
            }
        };

        let context = client.use_context.clone();

        if let Some(team_uuid) = Self::context_team_uuid(&context) {
            if !self.is_subscribed_to_team(&client_uuid, team_uuid) {
                self.send_to(addr, "401 Unauthorized: Not subscribed to team");
                return;
            }
        }

        match context {
            UseContext::Global => {
                let mut response = String::from("200 LIST_TEAMS");
                for team in self.db.teams.values() {
                    response.push_str(&format!(
                        "|{}:{}:{}",
                        team.uuid, team.name, team.description
                    ));
                }
                self.send_to(addr, &response);
            }
            UseContext::Team(team_uuid) => {
                let team = match self.db.teams.get(&team_uuid) {
                    Some(t) => t,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Team");
                        return;
                    }
                };

                let mut response = String::from("200 LIST_CHANNELS");
                for channel in team.channels.values() {
                    response.push_str(&format!(
                        "|{}:{}:{}",
                        channel.uuid, channel.name, channel.description
                    ));
                }
                self.send_to(addr, &response);
            }
            UseContext::Channel(team_uuid, channel_uuid) => {
                let team = match self.db.teams.get(&team_uuid) {
                    Some(t) => t,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Team");
                        return;
                    }
                };

                let channel = match team.channels.get(&channel_uuid) {
                    Some(c) => c,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Channel");
                        return;
                    }
                };

                let mut response = String::from("200 LIST_THREADS");
                for thread in channel.threads.values() {
                    response.push_str(&format!(
                        "|{}:{}:{}:{}:{}",
                        thread.uuid,
                        thread.author_uuid,
                        thread.timestamp,
                        thread.title,
                        thread.message
                    ));
                }
                self.send_to(addr, &response);
            }
            UseContext::Thread(team_uuid, channel_uuid, thread_uuid) => {
                let team = match self.db.teams.get(&team_uuid) {
                    Some(t) => t,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Team");
                        return;
                    }
                };

                let channel = match team.channels.get(&channel_uuid) {
                    Some(c) => c,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Channel");
                        return;
                    }
                };

                let thread = match channel.threads.get(&thread_uuid) {
                    Some(t) => t,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Thread");
                        return;
                    }
                };

                let mut response = String::from("200 LIST_REPLIES");
                for reply in &thread.replies {
                    response.push_str(&format!(
                        "|{}:{}:{}:{}",
                        thread.uuid, reply.sender_uuid, reply.timestamp, reply.body
                    ));
                }
                self.send_to(addr, &response);
            }
        }
    }

    fn cmd_info(&mut self, addr: SocketAddr) {
        let client = match self.clients.get(&addr) {
            Some(c) => c,
            None => return,
        };

        let client_uuid = match &client.uuid {
            Some(uuid) => uuid.clone(),
            None => {
                self.send_to(addr, "401 Unauthorized");
                return;
            }
        };

        let context = client.use_context.clone();

        if let Some(team_uuid) = Self::context_team_uuid(&context) {
            if !self.is_subscribed_to_team(&client_uuid, team_uuid) {
                self.send_to(addr, "401 Unauthorized: Not subscribed to team");
                return;
            }
        }

        match context {
            UseContext::Global => {
                let user = self.db.users.get(&client_uuid).unwrap();
                let status = if user.is_connected { "1" } else { "0" };
                self.send_to(
                    addr,
                    &format!("200 INFO_USER|{}|{}|{status}", user.uuid, user.name),
                );
            }
            UseContext::Team(team_uuid) => {
                let team = match self.db.teams.get(&team_uuid) {
                    Some(t) => t,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Team");
                        return;
                    }
                };

                self.send_to(
                    addr,
                    &format!("200 INFO_TEAM|{}|{}|{}", team.uuid, team.name, team.description),
                );
            }
            UseContext::Channel(team_uuid, channel_uuid) => {
                let team = match self.db.teams.get(&team_uuid) {
                    Some(t) => t,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Team");
                        return;
                    }
                };

                let channel = match team.channels.get(&channel_uuid) {
                    Some(c) => c,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Channel");
                        return;
                    }
                };

                self.send_to(
                    addr,
                    &format!(
                        "200 INFO_CHANNEL|{}|{}|{}",
                        channel.uuid, channel.name, channel.description
                    ),
                );
            }
            UseContext::Thread(team_uuid, channel_uuid, thread_uuid) => {
                let team = match self.db.teams.get(&team_uuid) {
                    Some(t) => t,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Team");
                        return;
                    }
                };

                let channel = match team.channels.get(&channel_uuid) {
                    Some(c) => c,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Channel");
                        return;
                    }
                };

                let thread = match channel.threads.get(&thread_uuid) {
                    Some(t) => t,
                    None => {
                        self.send_to(addr, "404 Not Found: Unknown Thread");
                        return;
                    }
                };

                self.send_to(
                    addr,
                    &format!(
                        "200 INFO_THREAD|{}|{}|{}|{}|{}",
                        thread.uuid,
                        thread.author_uuid,
                        thread.timestamp,
                        thread.title,
                        thread.message
                    ),
                );
            }
        }
    }

    fn cmd_subscribed(&mut self, addr: SocketAddr, args: &[String]) {
        let client_uuid = match self.get_client_uuid(addr) {
            Some(uuid) => uuid,
            None => {
                self.send_to(addr, "401 Unauthorized");
                return;
            }
        };

        if args.len() == 1 {
            let mut response = String::from("200 SUBSCRIBED_TEAMS");
            for team in self.db.teams.values() {
                if team.subscribers.iter().any(|u| u == &client_uuid) {
                    response.push_str(&format!(
                        "|{}:{}:{}",
                        team.uuid, team.name, team.description
                    ));
                }
            }
            self.send_to(addr, &response);
        } else if args.len() == 2 {
            let team_uuid = &args[1];
            let team = match self.db.teams.get(team_uuid) {
                Some(t) => t,
                None => {
                    self.send_to(addr, "404 Not Found: Unknown Team");
                    return;
                }
            };

            if !team.subscribers.iter().any(|u| u == &client_uuid) {
                self.send_to(addr, "401 Unauthorized: Not subscribed to team");
                return;
            }

            let mut response = String::from("200 SUBSCRIBED_USERS");
            for sub_uuid in &team.subscribers {
                if let Some(user) = self.db.users.get(sub_uuid) {
                    let status = if user.is_connected { "1" } else { "0" };
                    response.push_str(&format!("|{}:{}:{status}", user.uuid, user.name));
                }
            }
            self.send_to(addr, &response);
        } else {
            self.send_to(addr, "400 Bad Request");
        }
    }
}
