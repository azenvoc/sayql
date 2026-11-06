//! Workspaces and groups: how connections are organised, kept apart from `connections.json` so an
//! older version rewriting that file can't lose it.
//!
//! A workspace is one company: AI apps see one workspace (see `mcp::scope`). A group holds the
//! databases that belong together in it, as whole connections or single databases. A connection
//! with no entry here is in Personal with no group, so everything saved before this file existed
//! stays where it was. A connection may also have a list of the databases it can use, for a
//! server shared with other companies (see `safety::databases`).

use super::Store;
use crate::error::AppResult;
use crate::safety::databases::DbLimit;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const LAYOUT_FILE: &str = "layout.json";
/// The workspace that always exists, and the only one an AI app set up without one sees.
pub const PERSONAL: &str = "personal";
const MAX_NAME_CHARS: usize = 60;
/// Colours a new workspace can get: `--color-workspace-N` in styles/tokens.css. 0 is Personal's.
const WORKSPACE_COLORS: u8 = 6;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: String,
    pub name: String,
    /// Which `--color-workspace-N` its tile uses.
    #[serde(default)]
    pub color: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Group {
    pub id: String,
    pub workspace_id: String,
    pub name: String,
}

/// Where one connection sits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Placement {
    pub workspace_id: String,
    #[serde(default)]
    pub group_id: Option<String>,
    /// The databases this connection may use. `None`: all of them.
    #[serde(default)]
    pub allowed_databases: Option<Vec<String>>,
}

/// One database of a connection, placed in a group of its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseGroup {
    pub connection_id: String,
    pub database: String,
    pub group_id: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub groups: Vec<Group>,
    #[serde(default)]
    pub placement: BTreeMap<String, Placement>,
    #[serde(default)]
    pub database_groups: Vec<DatabaseGroup>,
    /// The workspace the team account's connections appear in. `None`: Personal.
    #[serde(default)]
    pub team_workspace: Option<String>,
}

/// What one `sayql-mcp` serves: a workspace, or one group in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub workspace: String,
    pub group: Option<String>,
}

impl Default for Scope {
    fn default() -> Self {
        Scope {
            workspace: PERSONAL.into(),
            group: None,
        }
    }
}

/// A name's clean form, or why it can't be used.
fn clean_name(name: &str, kind: &str) -> AppResult<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(format!("Enter a name for the {kind}.").into());
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("A {kind}'s name can be at most {MAX_NAME_CHARS} characters.").into());
    }
    Ok(name.to_string())
}

/// An id made from the name, readable in `sayql-mcp --workspace acme-logistics`, and not in
/// `taken`. It stays the same when the name changes.
fn new_id(name: &str, fallback: &str, taken: &[&str]) -> String {
    let slug = name
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let base = if slug.is_empty() {
        fallback.to_string()
    } else {
        slug
    };
    let mut id = base.clone();
    let mut n = 1;
    while taken.contains(&id.as_str()) {
        n += 1;
        id = format!("{base}-{n}");
    }
    id
}

impl Layout {
    /// A layout always has Personal, first.
    fn with_personal(mut self) -> Self {
        if !self.workspaces.iter().any(|w| w.id == PERSONAL) {
            self.workspaces.insert(
                0,
                Workspace {
                    id: PERSONAL.into(),
                    name: "Personal".into(),
                    color: 0,
                },
            );
        }
        self
    }

    pub fn has_workspace(&self, id: &str) -> bool {
        self.workspaces.iter().any(|w| w.id == id)
    }

    fn name_taken(names: impl Iterator<Item = impl AsRef<str>>, name: &str) -> bool {
        let mut names = names;
        names.any(|n| n.as_ref().to_lowercase() == name.to_lowercase())
    }

    /// The workspace `connection_id` is in. A team connection is in the team's workspace.
    pub fn workspace_of(&self, connection_id: &str) -> &str {
        if connection_id.starts_with(crate::team::ID_PREFIX) {
            return self.team_workspace.as_deref().unwrap_or(PERSONAL);
        }
        self.placement
            .get(connection_id)
            .map_or(PERSONAL, |p| p.workspace_id.as_str())
    }

    /// The databases a connection may use, whoever is asking.
    pub fn limit_of(&self, connection_id: &str) -> DbLimit {
        DbLimit::new(
            self.placement
                .get(connection_id)
                .and_then(|p| p.allowed_databases.clone()),
            vec![],
        )
    }

    /// What `scope` may reach of a connection: `None` when it's outside.
    pub fn reach(&self, scope: &Scope, connection_id: &str) -> Option<DbLimit> {
        if self.workspace_of(connection_id) != scope.workspace {
            return None;
        }
        let allowed = self
            .placement
            .get(connection_id)
            .and_then(|p| p.allowed_databases.clone());
        let Some(group) = &scope.group else {
            return Some(DbLimit::new(allowed, vec![]));
        };
        let entries = || {
            self.database_groups
                .iter()
                .filter(|d| d.connection_id == connection_id)
        };
        let whole = self
            .placement
            .get(connection_id)
            .is_some_and(|p| p.group_id.as_ref() == Some(group));
        if whole {
            // Its own databases, less any placed in another group.
            let elsewhere = entries()
                .filter(|d| &d.group_id != group)
                .map(|d| d.database.clone())
                .collect();
            return Some(DbLimit::new(allowed, elsewhere));
        }
        let here: Vec<String> = entries()
            .filter(|d| &d.group_id == group)
            .map(|d| d.database.clone())
            .filter(|d| {
                allowed
                    .as_ref()
                    .is_none_or(|a| a.iter().any(|x| x.eq_ignore_ascii_case(d)))
            })
            .collect();
        (!here.is_empty()).then(|| DbLimit::new(Some(here), vec![]))
    }

    /// The name of the group `scope` would show a connection under, for AI apps.
    pub fn group_name_of(&self, connection_id: &str) -> Option<&str> {
        let id = self.placement.get(connection_id)?.group_id.as_ref()?;
        self.groups
            .iter()
            .find(|g| &g.id == id)
            .map(|g| g.name.as_str())
    }

    pub fn create_workspace(&mut self, name: &str) -> AppResult<Workspace> {
        let name = clean_name(name, "workspace")?;
        if Self::name_taken(self.workspaces.iter().map(|w| &w.name), &name) {
            return Err("You already have a workspace with that name.".into());
        }
        let taken: Vec<&str> = self.workspaces.iter().map(|w| w.id.as_str()).collect();
        let color = (self.workspaces.len() % usize::from(WORKSPACE_COLORS)) as u8 + 1;
        let workspace = Workspace {
            id: new_id(&name, "workspace", &taken),
            name,
            color: color.min(WORKSPACE_COLORS),
        };
        self.workspaces.push(workspace.clone());
        Ok(workspace)
    }

    pub fn rename_workspace(&mut self, id: &str, name: &str) -> AppResult<()> {
        let name = clean_name(name, "workspace")?;
        let others = self.workspaces.iter().filter(|w| w.id != id);
        if Self::name_taken(others.map(|w| &w.name), &name) {
            return Err("You already have a workspace with that name.".into());
        }
        let workspace = self
            .workspaces
            .iter_mut()
            .find(|w| w.id == id)
            .ok_or("That workspace no longer exists.")?;
        workspace.name = name;
        Ok(())
    }

    /// The connections saved in a workspace, for "move them or delete them".
    pub fn connections_in(&self, workspace_id: &str) -> Vec<String> {
        self.placement
            .iter()
            .filter(|(_, p)| p.workspace_id == workspace_id)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Removes a workspace and its groups. Its connections move to `move_to`; with `None` the
    /// caller has already deleted them, the same way as deleting one connection.
    pub fn delete_workspace(&mut self, id: &str, move_to: Option<&str>) -> AppResult<()> {
        if id == PERSONAL {
            return Err("The Personal workspace can't be deleted.".into());
        }
        if !self.has_workspace(id) {
            return Err("That workspace no longer exists.".into());
        }
        if let Some(to) = move_to {
            if to == id || !self.has_workspace(to) {
                return Err("Choose another workspace to move its connections to.".into());
            }
            for connection in self.connections_in(id) {
                self.move_connection(&connection, to, None)?;
            }
        }
        let groups: Vec<String> = self
            .groups
            .iter()
            .filter(|g| g.workspace_id == id)
            .map(|g| g.id.clone())
            .collect();
        for group in groups {
            self.delete_group(&group);
        }
        self.placement.retain(|_, p| p.workspace_id != id);
        self.workspaces.retain(|w| w.id != id);
        if self.team_workspace.as_deref() == Some(id) {
            self.team_workspace = None;
        }
        Ok(())
    }

    pub fn create_group(&mut self, workspace_id: &str, name: &str) -> AppResult<Group> {
        let name = clean_name(name, "group")?;
        if !self.has_workspace(workspace_id) {
            return Err("That workspace no longer exists.".into());
        }
        let here = self
            .groups
            .iter()
            .filter(|g| g.workspace_id == workspace_id);
        if Self::name_taken(here.map(|g| &g.name), &name) {
            return Err("This workspace already has a group with that name.".into());
        }
        let taken: Vec<&str> = self.groups.iter().map(|g| g.id.as_str()).collect();
        let group = Group {
            id: new_id(&name, "group", &taken),
            workspace_id: workspace_id.to_string(),
            name,
        };
        self.groups.push(group.clone());
        Ok(group)
    }

    pub fn rename_group(&mut self, id: &str, name: &str) -> AppResult<()> {
        let name = clean_name(name, "group")?;
        let workspace = self
            .groups
            .iter()
            .find(|g| g.id == id)
            .ok_or("That group no longer exists.")?
            .workspace_id
            .clone();
        let others = self
            .groups
            .iter()
            .filter(|g| g.workspace_id == workspace && g.id != id);
        if Self::name_taken(others.map(|g| &g.name), &name) {
            return Err("This workspace already has a group with that name.".into());
        }
        if let Some(group) = self.groups.iter_mut().find(|g| g.id == id) {
            group.name = name;
        }
        Ok(())
    }

    /// Removes a group. Its connections and databases stay, with no group.
    pub fn delete_group(&mut self, id: &str) {
        self.groups.retain(|g| g.id != id);
        self.database_groups.retain(|d| d.group_id != id);
        for placement in self.placement.values_mut() {
            if placement.group_id.as_deref() == Some(id) {
                placement.group_id = None;
            }
        }
    }

    fn check_group(&self, workspace_id: &str, group_id: Option<&str>) -> AppResult<()> {
        if !self.has_workspace(workspace_id) {
            return Err("That workspace no longer exists.".into());
        }
        if let Some(group) = group_id
            && !self
                .groups
                .iter()
                .any(|g| g.id == group && g.workspace_id == workspace_id)
        {
            return Err("That group isn't in this workspace.".into());
        }
        Ok(())
    }

    /// Puts a connection in a workspace and group. Its single databases leave their groups when
    /// it changes workspace, since a group holds one workspace's databases.
    pub fn move_connection(
        &mut self,
        connection_id: &str,
        workspace_id: &str,
        group_id: Option<&str>,
    ) -> AppResult<()> {
        self.check_group(workspace_id, group_id)?;
        if self.workspace_of(connection_id) != workspace_id {
            self.database_groups
                .retain(|d| d.connection_id != connection_id);
        }
        let allowed_databases = self
            .placement
            .get(connection_id)
            .and_then(|p| p.allowed_databases.clone());
        self.placement.insert(
            connection_id.to_string(),
            Placement {
                workspace_id: workspace_id.to_string(),
                group_id: group_id.map(str::to_string),
                allowed_databases,
            },
        );
        Ok(())
    }

    /// Sets which databases a connection may use (`None`: all of them). Single databases no
    /// longer allowed leave their groups.
    pub fn set_allowed_databases(&mut self, connection_id: &str, allowed: Option<Vec<String>>) {
        let workspace_id = self.workspace_of(connection_id).to_string();
        let placement = self
            .placement
            .entry(connection_id.to_string())
            .or_insert(Placement {
                workspace_id,
                group_id: None,
                allowed_databases: None,
            });
        placement.allowed_databases = allowed.clone();
        if let Some(allowed) = allowed {
            self.database_groups.retain(|d| {
                d.connection_id != connection_id
                    || allowed.iter().any(|a| a.eq_ignore_ascii_case(&d.database))
            });
        }
    }

    /// Puts one database of a connection in a group, or back with its connection (`None`).
    pub fn set_database_group(
        &mut self,
        connection_id: &str,
        database: &str,
        group_id: Option<&str>,
    ) -> AppResult<()> {
        self.check_group(
            self.workspace_of(connection_id).to_string().as_str(),
            group_id,
        )?;
        self.database_groups
            .retain(|d| d.connection_id != connection_id || d.database != database);
        if let Some(group) = group_id {
            self.database_groups.push(DatabaseGroup {
                connection_id: connection_id.to_string(),
                database: database.to_string(),
                group_id: group.to_string(),
            });
        }
        Ok(())
    }

    pub fn set_team_workspace(&mut self, workspace_id: &str) -> AppResult<()> {
        self.check_group(workspace_id, None)?;
        self.team_workspace = Some(workspace_id.to_string());
        // Team connections placed in a group of the old workspace go back to having none.
        let team: Vec<String> = self
            .placement
            .keys()
            .filter(|id| id.starts_with(crate::team::ID_PREFIX))
            .cloned()
            .collect();
        for id in team {
            self.move_connection(&id, workspace_id, None)?;
        }
        Ok(())
    }

    /// Forgets a deleted connection.
    fn forget(&mut self, connection_id: &str) {
        self.placement.remove(connection_id);
        self.database_groups
            .retain(|d| d.connection_id != connection_id);
    }
}

impl Store {
    /// The saved layout. Everything that decides what an AI can reach reads this fresh, and an
    /// unreadable file is an error, never an empty layout.
    pub fn layout(&self) -> AppResult<Layout> {
        Ok(self.read::<Layout>(LAYOUT_FILE)?.with_personal())
    }

    /// Changes the layout under the file's lock (see [`Store::update`]) and returns the result.
    pub fn update_layout(
        &self,
        change: impl FnOnce(&mut Layout) -> AppResult<()>,
    ) -> AppResult<Layout> {
        self.update(LAYOUT_FILE, |saved: &mut Layout| {
            *saved = std::mem::take(saved).with_personal();
            change(saved)?;
            Ok(saved.clone())
        })
    }

    pub(super) fn forget_layout(&self, connection_id: &str) -> AppResult<()> {
        self.update_layout(|l| {
            l.forget(connection_id);
            Ok(())
        })
        .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::new(std::env::temp_dir().join(format!("sayql-layout-{}", uuid::Uuid::new_v4())))
    }

    /// Acme with two groups: `billing` in Portside, and two single databases of `shared`.
    fn acme() -> Layout {
        let mut l = Layout::default().with_personal();
        l.create_workspace("Acme Logistics").unwrap();
        l.create_group("acme-logistics", "Portside").unwrap();
        l.create_group("acme-logistics", "Meridian").unwrap();
        l.move_connection("billing", "acme-logistics", Some("portside"))
            .unwrap();
        l.move_connection("shared", "acme-logistics", Some("meridian"))
            .unwrap();
        l.set_database_group("shared", "portside_orders", Some("portside"))
            .unwrap();
        l
    }

    fn scope(workspace: &str, group: Option<&str>) -> Scope {
        Scope {
            workspace: workspace.into(),
            group: group.map(str::to_string),
        }
    }

    #[test]
    fn with_no_file_everything_is_in_personal_and_open() {
        let l = store().layout().unwrap();
        assert_eq!(l.workspaces.len(), 1);
        assert_eq!(l.workspaces[0].id, PERSONAL);
        assert_eq!(l.workspace_of("old-connection"), PERSONAL);
        assert_eq!(
            l.reach(&Scope::default(), "old-connection"),
            Some(DbLimit::default())
        );
    }

    #[test]
    fn a_placement_that_refers_to_a_connection_no_longer_saved_is_harmless() {
        let l = acme();
        assert_eq!(l.workspace_of("billing"), "acme-logistics");
        assert_eq!(l.reach(&Scope::default(), "billing"), None);
    }

    #[test]
    fn a_connection_outside_the_workspace_is_out_of_reach() {
        let l = acme();
        assert!(l.reach(&scope("acme-logistics", None), "billing").is_some());
        assert_eq!(l.reach(&scope("harbor", None), "billing"), None);
        assert_eq!(l.reach(&Scope::default(), "billing"), None);
    }

    #[test]
    fn a_group_scope_reaches_its_connections_and_its_single_databases_only() {
        let l = acme();
        let portside = scope("acme-logistics", Some("portside"));
        let meridian = scope("acme-logistics", Some("meridian"));
        assert_eq!(l.reach(&portside, "billing"), Some(DbLimit::default()));
        assert_eq!(l.reach(&meridian, "billing"), None);
        // `shared` is Meridian's, except the database placed in Portside.
        let single = l.reach(&portside, "shared").unwrap();
        assert!(single.allows("portside_orders"));
        assert!(!single.allows("meridian_orders"));
        let rest = l.reach(&meridian, "shared").unwrap();
        assert!(rest.allows("meridian_orders"));
        assert!(!rest.allows("portside_orders"));
    }

    #[test]
    fn a_group_never_widens_the_allowed_databases() {
        let mut l = acme();
        l.set_allowed_databases("shared", Some(vec!["meridian_orders".into()]));
        // The single database that's no longer allowed left its group, too.
        assert!(l.database_groups.is_empty());
        let meridian = scope("acme-logistics", Some("meridian"));
        assert!(!l.reach(&meridian, "shared").unwrap().allows("other"));
        l.set_database_group("shared", "other", Some("portside"))
            .unwrap();
        let portside = scope("acme-logistics", Some("portside"));
        assert_eq!(l.reach(&portside, "shared"), None);
    }

    #[test]
    fn moving_to_another_workspace_leaves_groups_but_keeps_the_allowed_list() {
        let mut l = acme();
        l.set_allowed_databases("shared", Some(vec!["portside_orders".into()]));
        l.create_workspace("Harbor").unwrap();
        l.move_connection("shared", "harbor", None).unwrap();
        assert!(l.database_groups.is_empty());
        let reach = l.reach(&scope("harbor", None), "shared").unwrap();
        assert!(!reach.allows("other"));
        assert!(reach.allows("portside_orders"));
    }

    #[test]
    fn names_are_trimmed_limited_and_unique_without_regard_to_case() {
        let mut l = acme();
        assert!(l.create_workspace("   ").is_err());
        assert!(l.create_workspace("acme logistics").is_err());
        assert!(l.create_workspace("PERSONAL").is_err());
        assert!(l.create_workspace(&"x".repeat(61)).is_err());
        assert!(l.create_workspace(&"x".repeat(60)).is_ok());
        assert!(l.create_group("acme-logistics", "PORTSIDE").is_err());
        // The same group name is fine in another workspace, with an id of its own.
        l.create_workspace("Harbor").unwrap();
        let again = l.create_group("harbor", "Portside").unwrap();
        assert_eq!(again.id, "portside-2");
        assert!(l.rename_group("portside", "meridian").is_err());
        assert!(l.rename_group("portside", "PORTSIDE").is_ok());
    }

    #[test]
    fn a_name_with_nothing_usable_in_it_still_gets_an_id() {
        let mut l = Layout::default().with_personal();
        assert_eq!(l.create_workspace("株式会社").unwrap().id, "workspace");
        assert_eq!(l.create_workspace("有限会社").unwrap().id, "workspace-2");
    }

    #[test]
    fn deleting_a_group_keeps_its_connections_with_no_group() {
        let mut l = acme();
        l.delete_group("portside");
        assert_eq!(l.placement["billing"].group_id, None);
        assert_eq!(l.workspace_of("billing"), "acme-logistics");
        assert!(l.database_groups.is_empty());
    }

    #[test]
    fn personal_can_not_be_deleted() {
        assert!(acme().delete_workspace(PERSONAL, None).is_err());
    }

    #[test]
    fn deleting_a_workspace_can_move_its_connections() {
        let mut l = acme();
        l.delete_workspace("acme-logistics", Some(PERSONAL))
            .unwrap();
        assert_eq!(l.workspace_of("billing"), PERSONAL);
        assert_eq!(l.placement["billing"].group_id, None);
        assert!(l.groups.is_empty());
        assert!(!l.has_workspace("acme-logistics"));
    }

    #[test]
    fn deleting_a_workspace_refuses_to_move_its_connections_nowhere() {
        let mut l = acme();
        assert!(
            l.delete_workspace("acme-logistics", Some("acme-logistics"))
                .is_err()
        );
        assert!(l.delete_workspace("acme-logistics", Some("gone")).is_err());
        assert_eq!(l.workspace_of("billing"), "acme-logistics");
    }

    #[test]
    fn a_group_must_be_in_the_workspace_it_is_used_with() {
        let mut l = acme();
        l.create_workspace("Harbor").unwrap();
        assert!(
            l.move_connection("billing", "harbor", Some("portside"))
                .is_err()
        );
        assert_eq!(l.workspace_of("billing"), "acme-logistics");
    }

    #[test]
    fn team_connections_follow_the_chosen_workspace() {
        let mut l = acme();
        assert_eq!(l.workspace_of("team:1"), PERSONAL);
        l.set_team_workspace("acme-logistics").unwrap();
        assert_eq!(l.workspace_of("team:1"), "acme-logistics");
        l.delete_workspace("acme-logistics", Some(PERSONAL))
            .unwrap();
        assert_eq!(l.workspace_of("team:1"), PERSONAL);
    }

    #[test]
    fn deleting_a_connection_forgets_its_place_and_saving_keeps_the_rest() {
        let s = store();
        s.update_layout(|l| {
            *l = acme();
            Ok(())
        })
        .unwrap();
        s.forget_agent("shared").unwrap();
        let l = s.layout().unwrap();
        assert!(!l.placement.contains_key("shared"));
        assert!(l.database_groups.is_empty());
        assert_eq!(l.workspace_of("billing"), "acme-logistics");
    }

    #[test]
    fn an_unreadable_file_is_an_error_not_an_empty_layout() {
        let s = store().without_repair();
        std::fs::write(s.dir().join(LAYOUT_FILE), "{not json").unwrap();
        assert!(s.layout().is_err());
        assert!(s.dir().join(LAYOUT_FILE).exists());
    }

    #[test]
    fn a_file_with_fields_this_version_does_not_know_still_loads() {
        let s = store();
        std::fs::write(
            s.dir().join(LAYOUT_FILE),
            r#"{"workspaces":[{"id":"a","name":"A","icon":"new"}],"futureField":1}"#,
        )
        .unwrap();
        let l = s.layout().unwrap();
        assert!(l.has_workspace("a"));
        assert!(l.has_workspace(PERSONAL));
    }
}
