//! What this `sayql-mcp` serves: one workspace, or one group in it. Set when the server starts
//! (`--workspace`, `--group`, or `SAYQL_WORKSPACE`, `SAYQL_GROUP`); with neither it serves
//! Personal only. Anything outside gets the same reply as something that doesn't exist, so an
//! app can't learn what else is saved. The layout is read fresh on every call, and one that can't
//! be read refuses every request.

use super::*;
use crate::safety::databases::DbLimit;
use crate::store::layout::{Layout, Scope};

/// What `run_query`, `describe_schema` and the rest say for a connection they can't reach, the
/// same as the store's message for one that isn't saved.
const NO_CONNECTION: &str = "Connection not found.";
pub(super) const WORKSPACE_VAR: &str = "SAYQL_WORKSPACE";
pub(super) const GROUP_VAR: &str = "SAYQL_GROUP";

/// The reply for a database outside the limit, whether or not it exists.
pub(super) fn no_database(name: &str) -> String {
    format!("This connection has no database named {name:?}; use one from list_databases.")
}

/// The scope from the command line, then the environment. A flag with no value is an error, so
/// a broken setup can't quietly serve the wrong workspace.
pub(super) fn scope_from(
    args: &[String],
    env: impl Fn(&str) -> Option<String>,
) -> Result<Scope, String> {
    let flag = |name: &str| -> Result<Option<String>, String> {
        let prefix = format!("{name}=");
        let mut found = None;
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            if arg == name {
                let value = it.next().ok_or(format!("{name} needs a value."))?;
                found = Some(value.clone());
            } else if let Some(value) = arg.strip_prefix(&prefix) {
                found = Some(value.to_string());
            }
        }
        Ok(found)
    };
    let pick =
        |value: Option<String>, var: &str| value.or_else(|| env(var)).filter(|v| !v.is_empty());
    let workspace = pick(flag("--workspace")?, WORKSPACE_VAR);
    let group = pick(flag("--group")?, GROUP_VAR);
    match (workspace, group) {
        (Some(workspace), group) => Ok(Scope { workspace, group }),
        (None, Some(_)) => Err("--group needs --workspace too.".into()),
        (None, None) => Ok(Scope::default()),
    }
}

/// Refuses to start on a workspace or group that isn't saved, with what to do about it.
pub(super) fn check_scope(layout: &Layout, scope: &Scope) -> Result<(), String> {
    if !layout.has_workspace(&scope.workspace) {
        return Err(format!(
            "No workspace has the id {:?}. Settings → AI apps in SayQL shows the setup for each one.",
            scope.workspace
        ));
    }
    if let Some(group) = &scope.group
        && !layout
            .groups
            .iter()
            .any(|g| &g.id == group && g.workspace_id == scope.workspace)
    {
        return Err(format!(
            "Workspace {:?} has no group with the id {group:?}. Settings → AI apps in SayQL shows the setup for each one.",
            scope.workspace
        ));
    }
    Ok(())
}

impl SayqlServer {
    /// The saved layout; one that can't be read refuses, never shows everything.
    pub(super) fn layout(&self) -> Result<Layout, String> {
        self.state
            .store
            .layout()
            .map_err(|_| SETTINGS_UNREADABLE.to_string())
    }

    /// What this server may reach of a saved connection. One outside the scope and one that
    /// isn't saved get the same error.
    pub(super) fn limit_for(&self, connection_id: &str) -> Result<DbLimit, String> {
        self.state
            .store
            .connection(connection_id)
            .map_err(|_| NO_CONNECTION.to_string())?;
        self.layout()?
            .reach(&self.scope, connection_id)
            .ok_or_else(|| NO_CONNECTION.to_string())
    }

    /// Whether a saved connection is in reach. Errors are only for an unreadable layout.
    pub(super) fn in_reach(&self, layout: &Layout, connection_id: &str) -> bool {
        layout.reach(&self.scope, connection_id).is_some()
    }

    /// The database to use for a request: the one named when the limit allows it, else the
    /// connection's default, else the first allowed. A limited connection always opens a
    /// database by name, so a query can't start in one it may not use.
    pub(super) async fn target_in_reach(
        &self,
        connection_id: &str,
        database: Option<String>,
    ) -> Result<Target, String> {
        let limit = self.limit_for(connection_id)?;
        if limit.is_open() {
            return Ok(Target::new(connection_id.to_string(), database));
        }
        let conn = self
            .state
            .store
            .connection(connection_id)
            .map_err(|_| NO_CONNECTION.to_string())?;
        if conn.kind == Kind::Sqlite {
            return Ok(Target::new(connection_id.to_string(), database));
        }
        let named = database
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty());
        if let Some(name) = named {
            return if limit.allows(&name) {
                Ok(Target::new(connection_id.to_string(), Some(name)))
            } else {
                Err(no_database(&name))
            };
        }
        let saved = conn.database.trim();
        let name = if !saved.is_empty() && limit.allows(saved) {
            saved.to_string()
        } else if let Some(first) = limit.first() {
            first.to_string()
        } else {
            self.first_listed(connection_id, &limit).await?
        };
        Ok(Target::new(connection_id.to_string(), Some(name)))
    }

    /// The server's current database, or its first one the limit allows, for a limit that only
    /// leaves some out.
    async fn first_listed(&self, connection_id: &str, limit: &DbLimit) -> Result<String, String> {
        let pool = self
            .state
            .target_pool(&Target::default_of(connection_id))
            .await
            .map_err(|e| e.to_string())?;
        let list = db::list_databases(&pool)
            .await
            .map_err(|e| e.to_string())?
            .ok_or("This connection has no databases to choose from.")?;
        list.current
            .filter(|c| limit.allows(c))
            .or_else(|| list.databases.into_iter().find(|d| limit.allows(d)))
            .ok_or_else(|| "This connection has no databases available here.".to_string())
    }

    /// A log entry with this server's workspace and group, then written. A log that can't be
    /// written doesn't fail the call.
    pub(super) fn write_log(&self, mut log: Activity) {
        log.workspace = Some(self.scope.workspace.clone());
        log.group.clone_from(&self.scope.group);
        let _ = self.state.store.log_app_activity(log);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn with_no_flags_or_environment_it_serves_personal_only() {
        assert_eq!(scope_from(&[], |_| None).unwrap(), Scope::default());
    }

    #[test]
    fn flags_win_over_the_environment() {
        let env = |k: &str| (k == WORKSPACE_VAR).then(|| "from-env".to_string());
        assert_eq!(scope_from(&[], env).unwrap().workspace, "from-env");
        let flags = args(&["--workspace", "acme", "--group=portside"]);
        let scope = scope_from(&flags, env).unwrap();
        assert_eq!(scope.workspace, "acme");
        assert_eq!(scope.group.as_deref(), Some("portside"));
    }

    #[test]
    fn a_broken_setup_is_an_error_not_a_default() {
        assert!(scope_from(&args(&["--workspace"]), |_| None).is_err());
        assert!(scope_from(&args(&["--group", "portside"]), |_| None).is_err());
    }

    #[test]
    fn a_workspace_or_group_that_is_not_saved_refuses_to_start() {
        let store = crate::store::Store::new(
            std::env::temp_dir().join(format!("sayql-scope-{}", uuid::Uuid::new_v4())),
        );
        let layout = store
            .update_layout(|l| {
                l.create_workspace("Acme")?;
                l.create_group("acme", "Portside").map(|_| ())
            })
            .unwrap();
        let scope = |w: &str, g: Option<&str>| Scope {
            workspace: w.into(),
            group: g.map(str::to_string),
        };
        assert!(check_scope(&layout, &scope("acme", Some("portside"))).is_ok());
        assert!(check_scope(&layout, &Scope::default()).is_ok());
        assert!(check_scope(&layout, &scope("gone", None)).is_err());
        assert!(check_scope(&layout, &scope("acme", Some("gone"))).is_err());
        assert!(check_scope(&layout, &scope("personal", Some("portside"))).is_err());
    }
}
