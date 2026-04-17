use std::{
    collections::HashMap,
    io::{ErrorKind, Read, Write},
    net::{SocketAddr, TcpListener},
    sync::atomic::Ordering,
};

use my_teams::ffi;

use crate::{
    client::{Client, UseContext},
    models::{Database, Message, User, generate_uuid},
};

const MAX_NAME_LENGTH: usize = 32;
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
                // TODO: FFi call to server_event_user_logged_out if the client was logged (62)
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
                ' ' if !in_quotes => {
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

    fn send_event_to_user(&mut self, user_uuid: &str, msg: &str) {
        for client in self.clients.values_mut() {
            if let Some(ref uuid) = client.uuid
                && uuid == user_uuid
            {
                client.queue_message(msg);
                break;
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
        let client = match self.clients.get_mut(&addr) {
            Some(c) => c,
            None => return,
        };

        if client.uuid.is_none() {
            client.queue_message("401 Unauthorized: Please login first");
            return;
        }

        match args.len() {
            1 => client.use_context = UseContext::Global,
            2 => client.use_context = UseContext::Team(args[1].clone()),
            3 => client.use_context = UseContext::Channel(args[1].clone(), args[2].clone()),
            4 => {
                client.use_context =
                    UseContext::Thread(args[1].clone(), args[2].clone(), args[3].clone())
            }
            _ => {
                client.queue_message("400 Bad Request: Invalid /use arguments");
                return;
            }
        }
        client.queue_message("200 Context Updated");
    }
}
