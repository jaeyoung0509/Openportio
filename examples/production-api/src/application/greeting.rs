use openportio_core::OpenportioError;

use crate::domain::greeting::{GreetingChannel, GreetingCommand, GreetingResult};

pub(crate) trait GreetingMessagePort: Send + Sync {
    fn render(&self, name: &str, actor: Option<&str>, channel: GreetingChannel) -> String;
}

#[derive(Debug, Clone)]
pub(crate) struct ServiceGreetingAdapter {
    service_name: String,
}

impl ServiceGreetingAdapter {
    pub(crate) fn new(service_name: String) -> Self {
        Self { service_name }
    }
}

impl GreetingMessagePort for ServiceGreetingAdapter {
    fn render(&self, name: &str, actor: Option<&str>, channel: GreetingChannel) -> String {
        let actor = actor.unwrap_or("anonymous");
        format!(
            "[{}][{}] hello, {} (actor={})",
            self.service_name,
            channel.as_str(),
            name,
            actor
        )
    }
}

#[derive(Debug, Clone)]
pub(crate) struct GreetingUseCase<P> {
    port: P,
}

impl<P> GreetingUseCase<P>
where
    P: GreetingMessagePort,
{
    pub(crate) fn new(port: P) -> Self {
        Self { port }
    }

    pub(crate) fn execute(
        &self,
        mut command: GreetingCommand,
    ) -> Result<GreetingResult, OpenportioError> {
        let normalized_name = command.name.trim();
        if normalized_name.is_empty() {
            return Err(OpenportioError::Validation(
                "name must not be empty".to_string(),
            ));
        }

        command.name = normalized_name.to_string();
        let actor = command.actor.take();
        let message = self
            .port
            .render(&command.name, actor.as_deref(), command.channel);

        Ok(GreetingResult {
            message,
            actor,
            channel: command.channel,
        })
    }
}
