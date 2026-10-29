import type { ReactNode } from "react";
import { Navigate, Route, Routes, useLocation } from "react-router";
import { useMe } from "@/lib/queries";
import { ErrorNote, Loading } from "@/components/ui";
import { AppLayout } from "./Layout";
import { LoginPage } from "@/features/auth/LoginPage";
import { SetupPage } from "@/features/auth/SetupPage";
import { InvitePage } from "@/features/auth/InvitePage";
import { DashboardPage } from "@/features/dashboard/DashboardPage";
import { SchedulesPage } from "@/features/schedules/SchedulesPage";
import { ScheduleDetailPage } from "@/features/schedules/ScheduleDetailPage";
import { ScheduleFormPage } from "@/features/schedules/ScheduleFormPage";
import { TeamsPage } from "@/features/teams/TeamsPage";
import { TeamDetailPage } from "@/features/teams/TeamDetailPage";
import { UsersPage } from "@/features/users/UsersPage";
import { IntegrationsPage } from "@/features/integrations/IntegrationsPage";
import { NotificationsPage } from "@/features/notifications/NotificationsPage";
import { AuditPage } from "@/features/audit/AuditPage";
import { SettingsPage } from "@/features/settings/SettingsPage";
import { ProfilePage } from "@/features/settings/ProfilePage";
import { SwapsPage } from "@/features/swaps/SwapsPage";
import { GuidePage } from "@/features/guide/GuidePage";

/** Signed-in users only. Others go to the login page and come back afterwards. */
export function RequireAuth({ children, needsOrg = true }: { children: ReactNode; needsOrg?: boolean }) {
  const { data: me, isLoading, error, refetch } = useMe();
  const location = useLocation();
  if (isLoading) return <div className="px-6"><Loading /></div>;
  if (error) return <div className="p-6"><ErrorNote error={error} retry={() => refetch()} /></div>;
  if (!me) return <Navigate to={`/login?return_to=${encodeURIComponent(location.pathname + location.search)}`} replace />;
  if (needsOrg && !me.active_organization_id) return <Navigate to="/setup" replace />;
  return <>{children}</>;
}

export function App() {
  return (
    <Routes>
      <Route path="/login" element={<LoginPage />} />
      <Route path="/invite/:token" element={<InvitePage />} />
      <Route path="/setup" element={<RequireAuth needsOrg={false}><SetupPage /></RequireAuth>} />
      <Route element={<RequireAuth><AppLayout /></RequireAuth>}>
        <Route index element={<DashboardPage />} />
        <Route path="schedules" element={<SchedulesPage />} />
        <Route path="schedules/new" element={<ScheduleFormPage />} />
        <Route path="schedules/:id" element={<ScheduleDetailPage />} />
        <Route path="schedules/:id/edit" element={<ScheduleFormPage />} />
        <Route path="swaps" element={<SwapsPage />} />
        <Route path="guide" element={<GuidePage />} />
        <Route path="teams" element={<TeamsPage />} />
        <Route path="teams/:id" element={<TeamDetailPage />} />
        <Route path="users" element={<UsersPage />} />
        <Route path="integrations" element={<IntegrationsPage />} />
        <Route path="notifications" element={<NotificationsPage />} />
        <Route path="audit" element={<AuditPage />} />
        <Route path="settings" element={<SettingsPage />} />
        <Route path="profile" element={<ProfilePage />} />
        <Route path="*" element={<NotFound />} />
      </Route>
    </Routes>
  );
}

function NotFound() {
  return (
    <div className="py-10">
      <h1 className="text-title font-semibold">Nothing here</h1>
      <p className="mt-3 text-lg">This page does not exist, or you do not have access to it.</p>
    </div>
  );
}
