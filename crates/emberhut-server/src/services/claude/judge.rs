//! The ask guard's judge model through the agent CLI: one turn with no tools and no MCP
//! servers, its instructions after the CLI's own system prompt, its answer's text out. Run on
//! the first configured account, or on the model provider the owner saved when none is
//! configured; given a minute.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::BoxFuture;

use super::cli::{CliSpawner, Launch};
use super::stream::{self, Said};
use crate::services::agent_provider::{ProviderEnv, ProviderStore};
use crate::services::guard::Judge;

/// How long the judge may take.
const TIMEOUT: Duration = Duration::from_mins(1);

/// The judge, as one CLI turn.
#[derive(Clone, Debug)]
pub struct CliJudge {
    spawner: Arc<dyn CliSpawner>,
    command: PathBuf,
    model: Option<String>,
    config_dir: Option<PathBuf>,
    provider: Option<ProviderStore>,
}

impl CliJudge {
    /// A judge running `command` (on `model`, with the CLI configuration `config_dir`) through
    /// `spawner`.
    #[must_use]
    pub fn new(
        spawner: Arc<dyn CliSpawner>,
        command: PathBuf,
        model: Option<String>,
        config_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            spawner,
            command,
            model,
            config_dir,
            provider: None,
        }
    }

    /// Runs the judge on the model provider saved in `store` (read at each judgement): for a
    /// server that configures no accounts.
    #[must_use]
    pub fn with_provider(mut self, store: ProviderStore) -> Self {
        self.provider = Some(store);
        self
    }

    fn launch(&self, provider: Option<&ProviderEnv>) -> Launch {
        let mut args: Vec<String> = [
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
            "--strict-mcp-config",
            "--permission-mode=dontAsk",
            "--max-turns=1",
        ]
        .map(str::to_owned)
        .to_vec();
        if let Some(model) = &self.model {
            args.push(format!("--model={model}"));
        }
        let mut env = Vec::new();
        if let Some(dir) = &self.config_dir {
            env.push(("CLAUDE_CONFIG_DIR".to_owned(), dir.display().to_string()));
        }
        let mut launch = Launch {
            program: self.command.clone(),
            args,
            env,
            env_remove: vec!["CLAUDECODE".to_owned()],
            env_remove_prefixed: Vec::new(),
            cwd: None,
            bridge: None,
        };
        if let Some(provider) = provider {
            provider.apply(&mut launch);
        }
        launch
    }
}

impl Judge for CliJudge {
    fn judge<'a>(&'a self, rules: &'a str, prompt: &'a str) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            let provider = match &self.provider {
                Some(store) => match store.env(&[]).await {
                    Ok(provider) => provider,
                    Err(error) => {
                        tracing::warn!(%error, "the ask guard's judge couldn't read the provider");
                        return None;
                    }
                },
                None => None,
            };
            let mut pipes = match self.spawner.spawn(&self.launch(provider.as_ref())) {
                Ok(pipes) => pipes,
                Err(error) => {
                    tracing::warn!(%error, "the ask guard's judge didn't start");
                    return None;
                }
            };
            if !pipes.send(stream::initialize("judge-1", rules))
                || !pipes.send(stream::user_message(prompt))
            {
                return None;
            }
            let answer = tokio::time::timeout(TIMEOUT, async {
                while let Some(line) = pipes.output.recv().await {
                    match Said::read(&line) {
                        Said::Result { is_error, text, .. } => {
                            return (!is_error).then_some(text);
                        }
                        Said::ControlRequest { id, subtype }
                            if !pipes.send(stream::unsupported(&id, &subtype)) =>
                        {
                            return None;
                        }
                        _ => {}
                    }
                }
                None
            })
            .await
            .ok()
            .flatten();
            pipes.close().await;
            answer
        })
    }
}
