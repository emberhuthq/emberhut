//! Every runner the server knows, as one [`RunnerApi`]: the ones its configuration names
//! (`--runner`) and the ones paired at run time.
//!
//! The configured runners come first, in their order, then the paired ones that the
//! configuration doesn't name. A name both give is the configured runner's: its address, words
//! and token are the configuration's, and the paired row of that name is neither reached nor
//! accepted as that runner's token. (Pairing refuses a name the configuration gives, so a clash
//! only comes from a configuration changed after the runner paired.)
//!
//! A request for a name neither knows goes to the configured runners, whose answer for an
//! unknown runner is the one every caller already expects.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use serde_json::Value;

use crate::services::runner_api::{ByteStream, RunnerApi, RunnerError, Verb};

/// The configured runners and the paired ones, as one.
#[derive(Debug)]
pub struct RunnerRegistry {
    configured: Arc<dyn RunnerApi>,
    paired: Arc<dyn RunnerApi>,
}

impl RunnerRegistry {
    /// The runners of `configured` and, under names it doesn't give, those of `paired`.
    #[must_use]
    pub fn new(configured: Arc<dyn RunnerApi>, paired: Arc<dyn RunnerApi>) -> Self {
        Self { configured, paired }
    }

    /// Whether the server's configuration names the runner `machine`.
    #[must_use]
    pub fn is_configured(&self, machine: &str) -> bool {
        self.configured
            .machines()
            .iter()
            .any(|known| known == machine)
    }

    /// The runners that answer for `machine`: the paired ones when only they know it.
    fn owner(&self, machine: &str) -> &dyn RunnerApi {
        if !self.is_configured(machine)
            && self.paired.machines().iter().any(|known| known == machine)
        {
            self.paired.as_ref()
        } else {
            self.configured.as_ref()
        }
    }
}

impl RunnerApi for RunnerRegistry {
    fn machines(&self) -> Vec<String> {
        let mut machines = self.configured.machines();
        for machine in self.paired.machines() {
            if !machines.contains(&machine) {
                machines.push(machine);
            }
        }
        machines
    }

    fn place(&self, machine: &str) -> String {
        self.owner(machine).place(machine)
    }

    fn address(&self, machine: &str) -> Option<String> {
        self.owner(machine).address(machine)
    }

    fn tokens(&self) -> BoxFuture<'_, BTreeMap<String, String>> {
        Box::pin(async move {
            let configured = self.configured.machines();
            let mut tokens: BTreeMap<String, String> = self
                .paired
                .tokens()
                .await
                .into_iter()
                .filter(|(machine, _)| !configured.contains(machine))
                .collect();
            tokens.extend(
                self.configured
                    .tokens()
                    .await
                    .into_iter()
                    .filter(|(machine, _)| configured.contains(machine)),
            );
            tokens
        })
    }

    fn call<'a>(
        &'a self,
        machine: &'a str,
        verb: Verb,
        path: &'a str,
        body: Option<Value>,
    ) -> BoxFuture<'a, Result<Value, RunnerError>> {
        self.owner(machine).call(machine, verb, path, body)
    }

    fn stream<'a>(
        &'a self,
        machine: &'a str,
        path: &'a str,
    ) -> BoxFuture<'a, Result<ByteStream, RunnerError>> {
        self.owner(machine).stream(machine, path)
    }

    fn upload<'a>(
        &'a self,
        machine: &'a str,
        path: &'a str,
        file: &'a Path,
    ) -> BoxFuture<'a, Result<Value, RunnerError>> {
        self.owner(machine).upload(machine, path, file)
    }

    fn download<'a>(
        &'a self,
        machine: &'a str,
        path: &'a str,
        to: &'a Path,
        limit: u64,
    ) -> BoxFuture<'a, Result<u64, RunnerError>> {
        self.owner(machine).download(machine, path, to, limit)
    }

    fn health<'a>(&'a self, machine: &'a str) -> BoxFuture<'a, Result<(), String>> {
        self.owner(machine).health(machine)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;

    /// Runners that answer every call with their own name, and note each call.
    #[derive(Debug)]
    struct Named {
        label: &'static str,
        runners: Vec<(&'static str, &'static str, &'static str)>,
        calls: Mutex<Vec<String>>,
    }

    impl Named {
        fn new(
            label: &'static str,
            runners: &[(&'static str, &'static str, &'static str)],
        ) -> Self {
            Self {
                label,
                runners: runners.to_vec(),
                calls: Mutex::default(),
            }
        }
    }

    impl RunnerApi for Named {
        fn machines(&self) -> Vec<String> {
            self.runners
                .iter()
                .map(|(name, ..)| (*name).to_owned())
                .collect()
        }

        fn place(&self, machine: &str) -> String {
            format!("{} {machine}", self.label)
        }

        fn address(&self, machine: &str) -> Option<String> {
            Some(format!("https://{machine}.{}.example.com", self.label))
        }

        fn tokens(&self) -> BoxFuture<'_, BTreeMap<String, String>> {
            let tokens = self
                .runners
                .iter()
                .map(|(name, _, token)| ((*name).to_owned(), (*token).to_owned()))
                .collect();
            Box::pin(async move { tokens })
        }

        fn call<'a>(
            &'a self,
            machine: &'a str,
            _verb: Verb,
            path: &'a str,
            _body: Option<Value>,
        ) -> BoxFuture<'a, Result<Value, RunnerError>> {
            self.calls.lock().unwrap().push(format!("{machine}{path}"));
            Box::pin(async move { Ok(json!({ "from": self.label })) })
        }

        fn stream<'a>(
            &'a self,
            machine: &'a str,
            _path: &'a str,
        ) -> BoxFuture<'a, Result<ByteStream, RunnerError>> {
            Box::pin(async move { Err(RunnerError::Unknown(machine.to_owned())) })
        }

        fn upload<'a>(
            &'a self,
            machine: &'a str,
            _path: &'a str,
            _file: &'a Path,
        ) -> BoxFuture<'a, Result<Value, RunnerError>> {
            Box::pin(async move { Err(RunnerError::Unknown(machine.to_owned())) })
        }

        fn download<'a>(
            &'a self,
            machine: &'a str,
            _path: &'a str,
            _to: &'a Path,
            _limit: u64,
        ) -> BoxFuture<'a, Result<u64, RunnerError>> {
            Box::pin(async move { Err(RunnerError::Unknown(machine.to_owned())) })
        }

        fn health<'a>(&'a self, _machine: &'a str) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async move { Err(self.label.to_owned()) })
        }
    }

    fn registry() -> (Arc<Named>, Arc<Named>, RunnerRegistry) {
        let configured = Arc::new(Named::new(
            "configured",
            &[("build", "", "build-flag"), ("runner", "", "runner-flag")],
        ));
        let paired = Arc::new(Named::new(
            "paired",
            &[
                ("build", "", "build-paired"),
                ("laptop", "", "laptop-paired"),
            ],
        ));
        let registry = RunnerRegistry::new(Arc::clone(&configured) as _, Arc::clone(&paired) as _);
        (configured, paired, registry)
    }

    #[tokio::test]
    async fn the_configured_runners_come_first_and_win_a_name() {
        let (configured, paired, registry) = registry();
        assert_eq!(registry.machines(), ["build", "runner", "laptop"]);
        assert!(registry.is_configured("build") && !registry.is_configured("laptop"));
        assert_eq!(registry.place("build"), "configured build");
        assert_eq!(registry.place("laptop"), "paired laptop");
        assert_eq!(
            registry.address("laptop").as_deref(),
            Some("https://laptop.paired.example.com")
        );
        let tokens = registry.tokens().await;
        assert_eq!(tokens["build"], "build-flag", "the configured token");
        assert_eq!(tokens["laptop"], "laptop-paired");
        assert_eq!(tokens.len(), 3);

        let answer = registry.call("laptop", Verb::Get, "/accounts", None).await;
        assert_eq!(answer.unwrap()["from"], "paired");
        let answer = registry.call("build", Verb::Get, "/accounts", None).await;
        assert_eq!(answer.unwrap()["from"], "configured");
        assert_eq!(registry.health("laptop").await.unwrap_err(), "paired");
        assert_eq!(*paired.calls.lock().unwrap(), ["laptop/accounts"]);
        assert_eq!(*configured.calls.lock().unwrap(), ["build/accounts"]);
    }

    #[tokio::test]
    async fn a_name_neither_knows_goes_to_the_configured_runners() {
        let (configured, _, registry) = registry();
        registry
            .call("elsewhere", Verb::Get, "/health", None)
            .await
            .unwrap();
        assert_eq!(*configured.calls.lock().unwrap(), ["elsewhere/health"]);
        assert_eq!(registry.place("elsewhere"), "configured elsewhere");
    }
}
