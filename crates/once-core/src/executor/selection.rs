use crate::{Action, Error, NetworkPolicy, RemoteExecution, Result};

/// The capacity an executor consumes, not its execution-platform identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResourceDomain {
    Host,
    /// Outstanding client requests. The service owns physical worker admission.
    Remote,
}

/// A resolved execution adapter. Aliases and tenancy remain in `RemoteExecution`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Executor {
    Local,
    Microsandbox,
    E2b,
    Daytona,
}

impl Executor {
    pub fn resolve(action: &Action) -> Result<Self> {
        let Action::RunCommand {
            remote: Some(binding),
            network,
            stdout_path,
            stderr_path,
            output_symlink_mode,
            ..
        } = action
        else {
            return Ok(Self::Local);
        };
        let executor = Self::from_binding(binding)?;
        if executor != Self::Local {
            if *network == NetworkPolicy::Deny {
                return Err(executor.unsupported("network denial"));
            }
            if stdout_path.is_some() || stderr_path.is_some() {
                return Err(executor.unsupported("stdout/stderr path redirection"));
            }
            if !output_symlink_mode.is_default() {
                return Err(executor.unsupported("non-default output symlink policy"));
            }
        }
        Ok(executor)
    }

    pub(crate) fn unsupported(self, requirement: &'static str) -> Error {
        Error::UnsupportedExecutorRequirement {
            executor: self.name().to_string(),
            requirement,
        }
    }

    pub fn from_binding(binding: &RemoteExecution) -> Result<Self> {
        match binding.executor.as_deref().unwrap_or(&binding.provider) {
            "local" if binding.environment.is_none() => Ok(Self::Local),
            "local" => Err(Error::RemoteProviderConfig {
                provider: binding.provider.clone(),
                message: "local execution cannot supply an image or template; select an executor that provides the requested environment".to_string(),
            }),
            "microsandbox" => Ok(Self::Microsandbox),
            "e2b" => Ok(Self::E2b),
            "daytona" => Ok(Self::Daytona),
            adapter => Err(Error::UnsupportedRemoteProvider {
                provider: adapter.to_string(),
            }),
        }
    }

    pub fn resource_domain(self) -> ResourceDomain {
        match self {
            Self::Local | Self::Microsandbox => ResourceDomain::Host,
            Self::E2b | Self::Daytona => ResourceDomain::Remote,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Microsandbox => "microsandbox",
            Self::E2b => "e2b",
            Self::Daytona => "daytona",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_resolution_uses_the_binding_not_the_alias() {
        let mut binding = RemoteExecution::provider("test-fleet");
        binding.executor = Some("e2b".to_string());
        assert_eq!(Executor::from_binding(&binding).unwrap(), Executor::E2b);
    }

    #[test]
    fn virtual_machines_share_the_host_domain() {
        assert_eq!(Executor::Local.resource_domain(), ResourceDomain::Host);
        assert_eq!(
            Executor::Microsandbox.resource_domain(),
            ResourceDomain::Host
        );
        assert_eq!(Executor::E2b.resource_domain(), ResourceDomain::Remote);
        assert_eq!(Executor::Daytona.resource_domain(), ResourceDomain::Remote);
    }

    #[test]
    fn unknown_adapters_fail_closed() {
        for name in ["unknown", "tuist", ""] {
            assert!(matches!(
                Executor::from_binding(&RemoteExecution::provider(name)),
                Err(Error::UnsupportedRemoteProvider { .. })
            ));
        }
    }
}
