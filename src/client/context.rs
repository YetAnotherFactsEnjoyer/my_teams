#[derive(Debug, Clone)]
pub struct Context {
    pub team_uuid: Option<String>,
    pub channel_uuid: Option<String>,
    pub thread_uuid: Option<String>,
}

impl Context {
    pub fn new() -> Self {
        Self {
            team_uuid: None,
            channel_uuid: None,
            thread_uuid: None,
        }
    }

    pub fn set(
        &mut self,
        team_uuid: Option<String>,
        channel_uuid: Option<String>,
        thread_uuid: Option<String>,
    ) {
        self.team_uuid = team_uuid;
        self.channel_uuid = channel_uuid;
        self.thread_uuid = thread_uuid;
    }

    pub fn clear(&mut self) {
        self.team_uuid = None;
        self.channel_uuid = None;
        self.thread_uuid = None;
    }
}
