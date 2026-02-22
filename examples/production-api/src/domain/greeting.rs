use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub(crate) enum GreetingChannel {
    Rest,
    Grpc,
}

impl GreetingChannel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Rest => "rest",
            Self::Grpc => "grpc",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GreetingCommand {
    pub(crate) name: String,
    pub(crate) actor: Option<String>,
    pub(crate) channel: GreetingChannel,
}

#[derive(Debug, Clone)]
pub(crate) struct GreetingResult {
    pub(crate) message: String,
    pub(crate) actor: Option<String>,
    pub(crate) channel: GreetingChannel,
}

#[derive(Debug, Serialize)]
pub(crate) struct GreetingResponse {
    pub(crate) message: String,
    pub(crate) actor: Option<String>,
    pub(crate) channel: &'static str,
}

impl GreetingResult {
    pub(crate) fn into_response(self) -> GreetingResponse {
        GreetingResponse {
            message: self.message,
            actor: self.actor,
            channel: self.channel.as_str(),
        }
    }
}
