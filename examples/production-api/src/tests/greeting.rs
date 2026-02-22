use crate::{
    application::greeting::{GreetingUseCase, ServiceGreetingAdapter},
    domain::greeting::{GreetingChannel, GreetingCommand},
};

#[test]
fn shared_greeting_use_case_supports_rest_and_grpc_channels() {
    let use_case = GreetingUseCase::new(ServiceGreetingAdapter::new("production-api".to_string()));

    let rest = use_case
        .execute(GreetingCommand {
            name: "Rust".to_string(),
            actor: Some("rest-user".to_string()),
            channel: GreetingChannel::Rest,
        })
        .expect("rest greeting should succeed");
    assert!(rest.message.contains("[rest]"));
    assert!(rest.message.contains("Rust"));

    let grpc = use_case
        .execute(GreetingCommand {
            name: "Rust".to_string(),
            actor: Some("grpc-user".to_string()),
            channel: GreetingChannel::Grpc,
        })
        .expect("grpc greeting should succeed");
    assert!(grpc.message.contains("[grpc]"));
    assert!(grpc.message.contains("Rust"));
}

#[test]
fn shared_greeting_use_case_rejects_blank_name() {
    let use_case = GreetingUseCase::new(ServiceGreetingAdapter::new("production-api".to_string()));

    let err = use_case
        .execute(GreetingCommand {
            name: "   ".to_string(),
            actor: Some("rest-user".to_string()),
            channel: GreetingChannel::Rest,
        })
        .expect_err("blank name should fail");

    let message = err.to_string();
    assert!(message.contains("name must not be empty"));
}
