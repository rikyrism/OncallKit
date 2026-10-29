export type Role = "owner" | "admin" | "scheduler" | "member";
export type Layer = "primary" | "secondary";
export type RotationUnit = "hours" | "days" | "weeks";

export type Permission =
  | "view_organization"
  | "manage_organization"
  | "transfer_ownership"
  | "manage_members"
  | "manage_teams"
  | "manage_schedules"
  | "manage_integrations"
  | "view_integrations"
  | "view_audit_logs";

export interface CurrentUser {
  user_id: string;
  session_id: string;
  email: string;
  display_name: string;
  avatar_url: string | null;
  discord_user_id: string | null;
  is_instance_admin: boolean;
  active_organization_id: string | null;
}

export interface Me {
  user: CurrentUser;
  csrf_token: string;
  memberships: { organization_id: string; name: string; slug: string; role: Role }[];
  active_organization_id: string | null;
  role: Role | null;
  permissions: Permission[];
  can_create_organization: boolean;
}

export interface Organization {
  id: string;
  name: string;
  slug: string;
  timezone: string;
  created_at: string;
}

export interface Member {
  user_id: string;
  email: string;
  display_name: string;
  avatar_url: string | null;
  role: Role;
  joined_at: string;
  team_ids: string[];
  discord_user_id: string | null;
}

export interface Invitation {
  id: string;
  email: string;
  role: Role;
  created_at: string;
  expires_at: string;
  accepted_at: string | null;
  revoked_at: string | null;
  invited_by_name: string | null;
}

export interface PendingInvitation {
  id: string;
  organization_id: string;
  organization_name: string;
  role: Role;
  expires_at: string;
}

export interface Team {
  id: string;
  name: string;
  description: string;
  created_at: string;
  member_ids: string[];
  active_schedule_count: number;
}

export interface ShiftSource {
  kind: "rotation" | "override";
  index?: number;
  override_id?: string;
}

export interface Shift {
  layer: Layer;
  user_id: string;
  start: string;
  end: string;
  source: ShiftSource;
}

export interface Transition {
  layer: Layer;
  at: string;
  previous_user_id: string | null;
  user_id: string | null;
  shift: Shift | null;
}

export interface OnCallNow {
  at: string;
  primary: Shift | null;
  secondary: Shift | null;
  next_rotation_at: string | null;
}

export interface RotationVersion {
  id: string;
  effective_from: string;
  interval: number;
  unit: RotationUnit;
  primary: string[];
  secondary: string[];
  created_at: string;
}

export type ScheduleState = "not_started" | "active" | "ended" | "paused" | "archived";

export interface Schedule {
  id: string;
  organization_id: string;
  team_id: string | null;
  name: string;
  description: string;
  timezone: string;
  start_at: string;
  end_at: string | null;
  status: "active" | "paused" | "archived";
  reminder_offsets_minutes: number[];
  notify_on_boundary: boolean;
  created_at: string;
  updated_at: string;
  start_local: string;
  end_local: string | null;
  state: ScheduleState;
  rotation: RotationVersion | null;
  on_call: OnCallNow;
}

export interface Override {
  id: string;
  layer: Layer;
  user_id: string;
  start_at: string;
  end_at: string;
  reason: string;
  created_at: string;
  created_by: string | null;
  canceled_at: string | null;
}

export interface ScheduleDetail extends Schedule {
  versions: RotationVersion[];
  overrides: Override[];
}

export interface Overview {
  schedules: {
    id: string;
    name: string;
    team_id: string | null;
    timezone: string;
    state: ScheduleState;
    on_call: OnCallNow;
  }[];
  upcoming: (Transition & { schedule_id: string; schedule_name: string; timezone: string; team_id: string | null })[];
}

export interface Integration {
  id: string;
  kind: "discord";
  name: string;
  team_id: string | null;
  enabled: boolean;
  events: string[];
  mention_users: boolean;
  message_prefix: string;
  max_attempts: number;
  webhook_hint: string;
  encrypted: boolean;
  last_delivery_status: string | null;
  last_delivery_at: string | null;
  created_at: string;
}

export type DeliveryStatus = "pending" | "sending" | "succeeded" | "failed" | "canceled";

export interface Delivery {
  id: string;
  status: DeliveryStatus;
  attempts: number;
  max_attempts: number;
  next_attempt_at: string;
  last_status_code: number | null;
  last_error: string | null;
  delivered_at: string | null;
  created_at: string;
  updated_at: string;
  event_type: string;
  event_scheduled_for: string;
  schedule_id: string | null;
  schedule_name: string | null;
  integration_id: string;
  integration_name: string;
}

export interface Attempt {
  attempt: number;
  started_at: string;
  finished_at: string;
  status_code: number | null;
  success: boolean;
  error: string | null;
}

export interface NotificationHealth {
  succeeded_24h: number;
  failed_24h: number;
  pending: number;
  retrying: number;
  integrations: number;
  integrations_enabled: number;
  last_success_at: string | null;
  last_failure_at: string | null;
}

export interface AuditEntry {
  id: string;
  action: string;
  target_type: string;
  target_id: string | null;
  metadata: Record<string, unknown>;
  created_at: string;
  actor_user_id: string | null;
  actor_name: string | null;
}

export interface Page<T> {
  items: T[];
  limit: number;
  offset: number;
  has_more: boolean;
}

export interface SessionInfo {
  id: string;
  user_agent: string | null;
  ip_address: string | null;
  created_at: string;
  last_seen_at: string;
  expires_at: string;
  current: boolean;
}

export interface CalendarFeed {
  id: string;
  schedule_id: string | null;
  schedule_name: string | null;
  created_at: string;
  last_used_at: string | null;
}

export interface CreatedCalendarFeed extends CalendarFeed {
  url: string;
  webcal_url: string;
}

export type SwapStatus = "pending" | "accepted" | "declined" | "canceled" | "expired";

export interface SwapRequest {
  id: string;
  schedule_id: string;
  schedule_name: string;
  timezone: string;
  layer: Layer;
  requester_id: string;
  target_user_id: string;
  start_at: string;
  end_at: string;
  return_start_at: string | null;
  return_end_at: string | null;
  reason: string;
  status: SwapStatus;
  created_at: string;
  decided_at: string | null;
  decided_by: string | null;
}
