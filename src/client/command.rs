#[derive(Debug, Clone)]
pub enum Command {
    Help,
    Login { user_name: String },
    Logout,
    Users,
    User { user_uuid: String },
    Send { user_uuid: String, body: String },
    Messages { user_uuid: String },
    Subscribe { team_uuid: String },
    Subscribed { team_uuid: Option<String> },
    Unsubscribe { team_uuid: String },
    Use {
        team_uuid: Option<String>,
        channel_uuid: Option<String>,
        thread_uuid: Option<String>,
    },
    Create { args: Vec<String> },
    List,
    Info,
}
