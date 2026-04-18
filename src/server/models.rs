use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{BufRead, BufReader, Write},
    time::UNIX_EPOCH,
};

pub fn generate_uuid() -> String {
    let time = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{time:x}")
}

#[derive(Clone)]
pub struct User {
    pub uuid: String,
    pub name: String,
    pub is_connected: bool,
}

#[derive(Clone)]
pub struct Message {
    pub sender_uuid: String,
    pub receiver_uuid: String,
    pub body: String,
    pub timestamp: u64,
}

#[derive(Clone)]
pub struct Thread {
    pub uuid: String,
    pub title: String,
    pub message: String,
    pub author_uuid: String,
    pub replies: Vec<Message>,
    pub timestamp: u64,
}

#[derive(Clone)]
pub struct Channel {
    pub uuid: String,
    pub name: String,
    pub description: String,
    pub threads: HashMap<String, Thread>,
}

#[derive(Clone)]
pub struct Team {
    pub uuid: String,
    pub name: String,
    pub description: String,
    pub subscribers: Vec<String>,
    pub channels: HashMap<String, Channel>,
}

#[derive(Default)]
pub struct Database {
    pub users: HashMap<String, User>,
    pub teams: HashMap<String, Team>,
    pub private_messages: Vec<Message>,
}

impl Database {
    pub fn save_to_file(&self, filepath: &str) -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(filepath)?;

        for user in self.users.values() {
            writeln!(
                file,
                "USER|{}|{}|{}",
                user.uuid,
                user.name.replace('\n', "\\n"),
                user.is_connected
            )?;
        }

        for msg in &self.private_messages {
            let safe_body = msg.body.replace('\n', "\\n");
            writeln!(
                file,
                "PM|{}|{}|{}|{}",
                msg.sender_uuid, msg.receiver_uuid, msg.timestamp, safe_body
            )?;
        }

        for team in self.teams.values() {
            writeln!(
                file,
                "TEAM|{}|{}|{}",
                team.uuid,
                team.name.replace('\n', "\\n"),
                team.description.replace('\n', "\\n")
            )?;

            for sub_uuid in &team.subscribers {
                writeln!(file, "SUB|{}|{sub_uuid}", team.uuid)?;
            }

            for channel in team.channels.values() {
                writeln!(
                    file,
                    "CHAN|{}|{}|{}|{}",
                    team.uuid,
                    channel.uuid,
                    channel.name.replace('\n', "\\n"),
                    channel.description.replace('\n', "\\n")
                )?;

                for thread in channel.threads.values() {
                    writeln!(
                        file,
                        "THRE|{}|{}|{}|{}|{}|{}|{}",
                        channel.uuid,
                        thread.uuid,
                        thread.author_uuid,
                        thread.timestamp,
                        thread.title.replace('\n', "\\n"),
                        thread.message.replace('\n', "\\n"),
                        team.uuid
                    )?;

                    for reply in &thread.replies {
                        writeln!(
                            file,
                            "REPL|{}|{}|{}|{}",
                            thread.uuid,
                            reply.sender_uuid,
                            reply.timestamp,
                            reply.body.replace('\n', "\\n")
                        )?;
                    }
                }
            }
        }

        println!("Data saved successfully in {filepath}");
        Ok(())
    }

    pub fn load_from_file(&mut self, filepath: &str) -> std::io::Result<()> {
        let file = File::open(filepath);
        if file.is_err() {
            println!("file {filepath} not found.");
            return Ok(());
        }

        let reader = BufReader::new(file?);

        for line in reader.lines() {
            let line = line?;
            let parts: Vec<&str> = line.split('|').collect();
            if parts.is_empty() {
                continue;
            }

            match parts[0] {
                "USER" if parts.len() == 4 => {
                    let user = User {
                        uuid: parts[1].to_string(),
                        name: parts[2].to_string().replace("\\n", "\n"),
                        is_connected: false,
                    };
                    my_teams::ffi::call_user_loaded(&user.uuid, &user.name);
                    self.users.insert(user.uuid.clone(), user);
                }
                "PM" if parts.len() == 5 => {
                    let msg = Message {
                        sender_uuid: parts[1].to_string(),
                        receiver_uuid: parts[2].to_string(),
                        timestamp: parts[3].parse().unwrap_or(0),
                        body: parts[4].replace("\\n", "\n"),
                    };
                    self.private_messages.push(msg);
                }
                "TEAM" if parts.len() == 4 => {
                    let team = Team {
                        uuid: parts[1].to_string(),
                        name: parts[2].replace("\\n", "\n"),
                        description: parts[3].replace("\\n", "\n"),
                        subscribers: Vec::new(),
                        channels: HashMap::new(),
                    };
                    self.teams.insert(team.uuid.clone(), team);
                }
                "SUB" if parts.len() == 3 => {
                    if let Some(team) = self.teams.get_mut(parts[1]) {
                        team.subscribers.push(parts[2].to_string());
                    }
                }
                "CHAN" if parts.len() == 5 => {
                    if let Some(team) = self.teams.get_mut(parts[1]) {
                        let chan = Channel {
                            uuid: parts[2].to_string(),
                            name: parts[3].to_string().replace("\\n", "\n"),
                            description: parts[4].replace("\\n", "\n"),
                            threads: HashMap::new(),
                        };
                        team.channels.insert(chan.uuid.clone(), chan);
                    }
                }
                "THRE" if parts.len() == 8 => {
                    if let Some(team) = self.teams.get_mut(parts[7])
                        && let Some(chan) = team.channels.get_mut(parts[1])
                    {
                        let thread = Thread {
                            uuid: parts[2].to_string(),
                            author_uuid: parts[3].to_string(),
                            timestamp: parts[4].parse().unwrap_or(0),
                            title: parts[5].replace("\\n", "\n"),
                            message: parts[6].replace("\\n", "\n"),
                            replies: Vec::new(),
                        };
                        chan.threads.insert(thread.uuid.clone(), thread);
                    }
                }
                "REPL" if parts.len() == 5 => {
                    let thread_uuid = parts[1];
                    'find_thread: for team in self.teams.values_mut() {
                        for chan in team.channels.values_mut() {
                            if let Some(thread) = chan.threads.get_mut(thread_uuid) {
                                thread.replies.push(Message {
                                    sender_uuid: parts[2].to_string(),
                                    receiver_uuid: thread_uuid.to_string(),
                                    timestamp: parts[3].parse().unwrap_or(0),
                                    body: parts[4].replace("\\n", "\n"),
                                });
                                break 'find_thread;
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        println!("Data loaded successfully from {filepath}");
        Ok(())
    }
}
