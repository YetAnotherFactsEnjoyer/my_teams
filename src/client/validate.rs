use crate::client::command::Command;
use crate::client::context::Context;

pub const MAX_NAME_LENGTH: usize = 32;
pub const MAX_DESCRIPTION_LENGTH: usize = 255;
pub const MAX_BODY_LENGTH: usize = 512;

fn validate_name(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("Name cannot be empty".to_string());
    }
    if value.len() > MAX_NAME_LENGTH {
        return Err(format!("Name too long (max {})", MAX_NAME_LENGTH));
    }
    Ok(())
}

fn validate_description(value: &str) -> Result<(), String> {
    if value.len() > MAX_DESCRIPTION_LENGTH {
        return Err(format!(
            "Description too long (max {})",
            MAX_DESCRIPTION_LENGTH
        ));
    }
    Ok(())
}

fn validate_body(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("Body cannot be empty".to_string());
    }
    if value.len() > MAX_BODY_LENGTH {
        return Err(format!("Body too long (max {})", MAX_BODY_LENGTH));
    }
    Ok(())
}

pub fn validate_command(cmd: &Command, ctx: &Context) -> Result<(), String> {
    match cmd {
        Command::Login { user_name } => validate_name(user_name),
        Command::User { user_uuid }
        | Command::Messages { user_uuid } => {
            if user_uuid.is_empty() {
                return Err("UUID cannot be empty".to_string());
            }
            Ok(())
        }
        Command::Send { user_uuid, body } => {
            if user_uuid.is_empty() {
                return Err("UUID cannot be empty".to_string());
            }
            validate_body(body)
        }
        Command::Subscribe { team_uuid } | Command::Unsubscribe { team_uuid } => {
            if team_uuid.is_empty() {
                return Err("team_uuid cannot be empty".to_string());
            }
            Ok(())
        }
        Command::Subscribed { team_uuid } => {
            if let Some(id) = team_uuid {
                if id.is_empty() {
                    return Err("team_uuid cannot be empty".to_string());
                }
            }
            Ok(())
        }
        Command::Use {
            team_uuid,
            channel_uuid,
            thread_uuid,
        } => {
            if team_uuid.is_none() && (channel_uuid.is_some() || thread_uuid.is_some()) {
                return Err("Cannot set channel/thread without team".to_string());
            }
            if channel_uuid.is_none() && thread_uuid.is_some() {
                return Err("Cannot set thread without channel".to_string());
            }
            Ok(())
        }
        Command::Create { args } => {
            match (&ctx.team_uuid, &ctx.channel_uuid, &ctx.thread_uuid) {
                (None, None, None) => {
                    if args.len() != 2 {
                        return Err(r#"/create requires "team_name" "team_description""#.to_string());
                    }
                    validate_name(&args[0])?;
                    validate_description(&args[1])?;
                    Ok(())
                }
                (Some(_), None, None) => {
                    if args.len() != 2 {
                        return Err(r#"/create requires "channel_name" "channel_description""#.to_string());
                    }
                    validate_name(&args[0])?;
                    validate_description(&args[1])?;
                    Ok(())
                }
                (Some(_), Some(_), None) => {
                    if args.len() != 2 {
                        return Err(r#"/create requires "thread_title" "thread_message""#.to_string());
                    }
                    validate_name(&args[0])?;
                    validate_body(&args[1])?;
                    Ok(())
                }
                (Some(_), Some(_), Some(_)) => {
                    if args.len() != 1 {
                        return Err(r#"/create requires "comment_body""#.to_string());
                    }
                    validate_body(&args[0])?;
                    Ok(())
                }
                _ => Err("Invalid context".to_string()),
            }
        }
        _ => Ok(()),
    }
}
