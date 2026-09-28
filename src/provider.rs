//! The service that answers Jev's questions, and what JevGate needs to know
//! to talk to it and to explain its failures.

/// One provider of TypeSafe's API.
#[derive(Debug)]
pub struct Service {
    /// The name people read in messages.
    pub label: &'static str,
    /// The API root; requests go to `<root>/v1/systemone`.
    pub api_root: &'static str,
    /// What to do when a request is refused for want of credits (HTTP 402).
    pub credits: &'static str,
}

/// TypeSafe itself. Its API documents no 402; its terms bill prepaid credits
/// that can refill automatically (MCA §8.2), managed in the console.
pub const TYPESAFE: Service = Service {
    label: "TypeSafe",
    api_root: "https://api.typesafe.ai",
    credits: "add credits or turn on auto-refill at https://console.typesafe.ai",
};

/// Where a check sends its requests, and the provider that answers there.
#[derive(Debug)]
pub struct Endpoint {
    pub service: &'static Service,
    root: String,
}

impl Endpoint {
    pub fn new(service: &'static Service) -> Self {
        Self::at(service, service.api_root)
    }

    /// An endpoint at another API root, such as a test server.
    pub fn at(service: &'static Service, root: &str) -> Self {
        Self {
            service,
            root: root.trim_end_matches('/').to_owned(),
        }
    }

    /// The URL questions are posted to.
    pub fn systemone(&self) -> String {
        format!("{}/v1/systemone", self.root)
    }
}
