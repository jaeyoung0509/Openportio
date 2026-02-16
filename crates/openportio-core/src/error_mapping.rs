#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainRestStatus {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    TooManyRequests,
    InternalServerError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DomainGrpcCode {
    InvalidArgument,
    Unauthenticated,
    PermissionDenied,
    NotFound,
    AlreadyExists,
    ResourceExhausted,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainErrorMapping {
    pub rest_status: DomainRestStatus,
    pub rest_code: &'static str,
    pub grpc_code: DomainGrpcCode,
    pub expose_message: bool,
    pub issue_type: Option<&'static str>,
}

pub trait DomainErrorDescriptor {
    fn domain_error_mapping(&self) -> DomainErrorMapping;
    fn domain_error_message(&self) -> &str;
}
