//! Role-based access control. Roles are per organization; every protected handler asks for a
//! [`Permission`] through `OrgContext::require`.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Member,
    Scheduler,
    Admin,
    Owner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Permission {
    ViewOrganization,
    ManageOrganization,
    TransferOwnership,
    ManageMembers,
    ManageTeams,
    ManageSchedules,
    ManageIntegrations,
    ViewIntegrations,
    ViewAuditLogs,
}

impl Role {
    pub fn parse(value: &str) -> Option<Role> {
        match value {
            "owner" => Some(Role::Owner),
            "admin" => Some(Role::Admin),
            "scheduler" => Some(Role::Scheduler),
            "member" => Some(Role::Member),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Scheduler => "scheduler",
            Role::Member => "member",
        }
    }

    pub fn allows(self, permission: Permission) -> bool {
        use Permission::*;
        match permission {
            ViewOrganization => true,
            ManageSchedules => self >= Role::Scheduler,
            ManageMembers | ManageTeams | ManageIntegrations | ViewIntegrations | ViewAuditLogs => self >= Role::Admin,
            ManageOrganization | TransferOwnership => self == Role::Owner,
        }
    }

    /// Whether a member with role `self` may give or take away `target`. Only owners can touch
    /// ownership; admins manage everyone below admin.
    pub fn can_assign(self, target: Role) -> bool {
        match self {
            Role::Owner => true,
            Role::Admin => target < Role::Admin,
            _ => false,
        }
    }

    pub fn permissions(self) -> Vec<&'static str> {
        use Permission::*;
        [
            (ViewOrganization, "view_organization"),
            (ManageOrganization, "manage_organization"),
            (TransferOwnership, "transfer_ownership"),
            (ManageMembers, "manage_members"),
            (ManageTeams, "manage_teams"),
            (ManageSchedules, "manage_schedules"),
            (ManageIntegrations, "manage_integrations"),
            (ViewIntegrations, "view_integrations"),
            (ViewAuditLogs, "view_audit_logs"),
        ]
        .into_iter()
        .filter(|(p, _)| self.allows(*p))
        .map(|(_, name)| name)
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Permission::*;

    #[test]
    fn members_can_only_view() {
        assert!(Role::Member.allows(ViewOrganization));
        for p in [ManageSchedules, ManageTeams, ManageMembers, ManageIntegrations, ViewAuditLogs, ManageOrganization] {
            assert!(!Role::Member.allows(p), "{p:?}");
        }
    }

    #[test]
    fn schedulers_manage_schedules_only() {
        assert!(Role::Scheduler.allows(ManageSchedules));
        assert!(!Role::Scheduler.allows(ManageTeams));
        assert!(!Role::Scheduler.allows(ManageIntegrations));
    }

    #[test]
    fn admins_manage_everything_but_the_organization() {
        for p in [ManageSchedules, ManageTeams, ManageMembers, ManageIntegrations, ViewAuditLogs] {
            assert!(Role::Admin.allows(p), "{p:?}");
        }
        assert!(!Role::Admin.allows(ManageOrganization));
        assert!(!Role::Admin.allows(TransferOwnership));
    }

    #[test]
    fn owners_can_do_everything() {
        for p in [
            ViewOrganization,
            ManageOrganization,
            TransferOwnership,
            ManageMembers,
            ManageTeams,
            ManageSchedules,
            ManageIntegrations,
            ViewIntegrations,
            ViewAuditLogs,
        ] {
            assert!(Role::Owner.allows(p));
        }
    }

    #[test]
    fn admins_cannot_create_admins_or_owners() {
        assert!(Role::Admin.can_assign(Role::Scheduler));
        assert!(!Role::Admin.can_assign(Role::Admin));
        assert!(!Role::Admin.can_assign(Role::Owner));
        assert!(Role::Owner.can_assign(Role::Admin));
        assert!(!Role::Scheduler.can_assign(Role::Member));
    }
}
