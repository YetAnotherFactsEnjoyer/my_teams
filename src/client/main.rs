use std::{
    io::{BufRead, BufReader, Write},
    net::TcpStream,
    sync::{Arc, Mutex},
    thread,
};

use my_teams::ffi;

struct ClientState {
    uuid: String,
    name: String,
}

fn handle_server_message(message: &str, state: &Arc<Mutex<ClientState>>) {
    let parts: Vec<&str> = message.split('|').collect();
    if parts.is_empty() {
        return;
    }

    match parts[0] {
        s if s.starts_with("200 Login OK") && parts.len() == 3 => {
            let uuid = parts[1];
            let name = parts[2];

            if let Ok(mut s) = state.lock() {
                s.uuid = uuid.to_string();
                s.name = name.to_string();
            }
            ffi::call_client_event_logged_in(uuid, name);
        }
        "200 Logout OK" => {
            if let Ok(mut s) = state.lock() {
                ffi::call_client_event_logged_out(&s.uuid, &s.name);
                s.uuid.clear();
                s.name.clear();
            }
        }
        "EVENT PM_RECEIVED" if parts.len() == 3 => {
            let sender_uuid = parts[1];
            let body = parts[2];
            ffi::call_client_event_private_message_received(sender_uuid, body);
        }
        "200 USERS" => {
            for user_data in parts.iter().skip(1) {
                let u_parts: Vec<&str> = user_data.split(':').collect();
                if u_parts.len() == 3 {
                    let status = u_parts[2].parse::<i32>().unwrap_or(0);
                    ffi::call_client_print_users(u_parts[0], u_parts[1], status);
                }
            }
        }
        "200 USER" if parts.len() == 4 => {
            let status = parts[3].parse::<i32>().unwrap_or(0);
            ffi::call_client_print_user(parts[1], parts[2], status);
        }
        "200 REPLY_CREATED" if parts.len() == 5 => {
            let ts = parts[3].parse::<u64>().unwrap_or(0);
            ffi::call_client_print_reply_created(parts[1], parts[2], ts, parts[4]);
        }
        s if s.starts_with("401") => {
            ffi::call_client_error_unauthorized();
        }
        s if s.starts_with("409") => {
            ffi::call_client_error_already_exist();
        }
        s if s.starts_with("404") => {
            if parts[0].contains("User") {
                ffi::call_client_error_unknown_user("unknown");
            } else if parts[0].contains("Team") {
                ffi::call_client_error_unknown_team("unknwon");
            }
        }
        _ => {
            println!("{message}");
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(flag) = args.get(1)
        && flag == "--help"
    {
        println!(
            "USAGE: ./myteams_cli ip port\n\n ip is the server ip address on which the server socket listens\n port is the port number on which the server socket listens"
        );
        std::process::exit(0);
    }
    if args.len() != 3 {
        println!(
            "USAGE: ./myteams_cli ip port\n\n ip is the server ip address on which the server socket listens\n port is the port number on which the server socket listens"
        );
        std::process::exit(84);
    }

    let ip = &args[1];
    let port = &args[2];
    let addr = format!("{ip}:{port}");

    let mut stream = match TcpStream::connect(&addr) {
        Ok(s) => s,
        Err(e) => {
            println!("Error connecting to server: {e}");
            std::process::exit(84);
        }
    };

    println!("Connected to server {addr}");

    let stream_clone = stream.try_clone().expect("Error cloning TcpStream");
    let state = Arc::new(Mutex::new(ClientState {
        uuid: String::new(),
        name: String::new(),
    }));
    let state_clone = Arc::clone(&state);

    thread::spawn(move || {
        let mut reader = BufReader::new(stream_clone);
        let mut line = String::new();

        while match reader.read_line(&mut line) {
            Ok(bytes) => bytes > 0,
            Err(_) => false,
        } {
            handle_server_message(line.trim_end(), &state_clone);
            line.clear();
        }
        println!("Connexion closed by server.");
        std::process::exit(0);
    });

    let stdin = std::io::stdin();
    for line_result in stdin.lock().lines() {
        let mut command = line_result.expect("Error reading stdin");

        if command.is_empty() {
            break;
        }

        if command.starts_with("/help") {
            println!(
                "/help : show help\n/login [”user_name”] : set the user_name used by client\n/logout : disconnect the client from the server\n/users : get the list of all users that exist on the domain\n/user [”user_uuid”] : get details about the requested user\n/send [”user_uuid”] [”message_body”] : send a message to specific user\n/messages [”user_uuid”] : list all messages exchanged with the specified user\n/subscribe [”team_uuid”] : subscribe to the events of a team and its sub directories (enable reception of all events from a team)\n/subscribed ?[”team_uuid”] : list all subscribed teams or list all users subscribed to a team\n/unsubscribe [”team_uuid”] : unsubscribe from a team\n/use ?[”team_uuid”] ?[”channel_uuid”] ?[”thread_uuid”] : Sets the command context to a team/channel/thread\n/create : based on the context, create the sub resource\n/list : based on the context, list all the sub resources\n/info : based on the context, display details of the current resource"
            )
        } else {
            command.push('\n');
            if stream.write_all(command.as_bytes()).is_err() {
                println!("Error sending command.");
                break;
            }
        }
    }
}
