import { afterEach, describe, expect, it, vi } from "vitest";
import { screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Route, Routes } from "react-router";
import { App } from "@/app/App";
import { AppLayout } from "@/app/Layout";
import { LoginPage } from "@/features/auth/LoginPage";
import { RotationPreview } from "@/features/schedules/ScheduleFormPage";
import { IntegrationsPage } from "@/features/integrations/IntegrationsPage";
import { NotificationsPage } from "@/features/notifications/NotificationsPage";
import { meFixture, mockApi, renderWithProviders } from "./utils";
import type { ScheduleForm } from "@/features/schedules/schema";

afterEach(() => vi.unstubAllGlobals());

const ADMIN = ["view_organization", "manage_members", "manage_teams", "manage_schedules", "manage_integrations", "view_integrations", "view_audit_logs"];

describe("authentication", () => {
  it("sends signed-out visitors to the login page and keeps where they were going", async () => {
    mockApi((url) => {
      if (url === "/me") return { status: 401, body: { error: { code: "unauthenticated", message: "authentication required" } } };
      if (url === "/auth/providers") return { status: 200, body: { google: true, dev_login: false } };
    });
    renderWithProviders(<App />, { route: "/schedules" });
    expect(await screen.findByRole("heading", { name: "Sign in" })).toBeInTheDocument();
    const google = screen.getByRole("link", { name: /continue with google/i });
    expect(google).toHaveAttribute("href", "/api/v1/auth/google/start?return_to=%2Fschedules");
  });

  it("explains why sign-in was refused", async () => {
    mockApi((url) => {
      if (url === "/me") return { status: 401 };
      if (url === "/auth/providers") return { status: 200, body: { google: true, dev_login: false } };
    });
    renderWithProviders(<Routes><Route path="/login" element={<LoginPage />} /></Routes>, { route: "/login?error=registration_closed" });
    expect(await screen.findByRole("alert")).toHaveTextContent(/only accepts invited people/);
  });

  it("sends people without an organization to setup", async () => {
    mockApi((url) => {
      if (url === "/me") return { status: 200, body: { ...meFixture("member", []), active_organization_id: null, role: null } };
      if (url === "/me/invitations") return { status: 200, body: [] };
    });
    renderWithProviders(<App />, { route: "/" });
    expect(await screen.findByRole("heading", { name: /create an organization/i })).toBeInTheDocument();
  });
});

describe("permission-dependent UI", () => {
  it("hides administration from members", async () => {
    mockApi((url) => (url === "/me" ? { status: 200, body: meFixture("member", ["view_organization"]) } : undefined));
    renderWithProviders(<Routes><Route path="/" element={<AppLayout><p>content</p></AppLayout>} /></Routes>);
    expect((await screen.findAllByRole("link", { name: "Schedules" })).length).toBeGreaterThan(0);
    expect(screen.queryByRole("link", { name: "Integrations" })).toBeNull();
    expect(screen.queryByRole("link", { name: "Audit logs" })).toBeNull();
  });

  it("shows administration to admins", async () => {
    mockApi((url) => (url === "/me" ? { status: 200, body: meFixture("admin", ADMIN) } : undefined));
    renderWithProviders(<Routes><Route path="/" element={<AppLayout><p>content</p></AppLayout>} /></Routes>);
    expect((await screen.findAllByRole("link", { name: "Integrations" })).length).toBeGreaterThan(0);
    expect(screen.getAllByRole("link", { name: "Audit logs" }).length).toBeGreaterThan(0);
  });
});

describe("rotation preview", () => {
  const values: ScheduleForm = {
    name: "DevOps",
    description: "",
    team_id: "",
    timezone: "Asia/Jakarta",
    start_local: "2026-11-03T09:00",
    open_ended: true,
    end_local: "",
    interval: 7,
    unit: "days",
    primary: ["u1", "u2"],
    secondary: [],
    reminder_offsets_minutes: [60],
    notify_on_boundary: true,
  };

  it("asks the server engine for shifts and lists them in the schedule's timezone", async () => {
    const calls = mockApi((url) => {
      if (url === "/members")
        return {
          status: 200,
          body: [
            { user_id: "u1", email: "r@a", display_name: "Riky", avatar_url: null, role: "owner", joined_at: "", team_ids: [] },
            { user_id: "u2", email: "a@a", display_name: "Andi", avatar_url: null, role: "member", joined_at: "", team_ids: [] },
          ],
        };
      if (url === "/schedules/preview")
        return {
          status: 200,
          body: {
            start_at: "2026-11-03T02:00:00Z",
            end_at: null,
            shifts: [
              { layer: "primary", user_id: "u1", start: "2026-11-03T02:00:00Z", end: "2026-11-10T02:00:00Z", source: { kind: "rotation", index: 0 } },
              { layer: "primary", user_id: "u2", start: "2026-11-10T02:00:00Z", end: "2026-11-17T02:00:00Z", source: { kind: "rotation", index: 1 } },
            ],
          },
        };
    });
    renderWithProviders(<RotationPreview values={values} />);
    expect(await screen.findByText(/3 Nov 2026, 09:00 to 10 Nov 2026, 09:00/)).toBeInTheDocument();
    expect(screen.getAllByText("Andi").length).toBeGreaterThan(0);
    const preview = calls.find((c) => c.url === "/schedules/preview")!;
    expect(JSON.parse(String(preview.init?.body))).toMatchObject({ start_local: "2026-11-03T09:00", end_local: null, rotation: { interval: 7, unit: "days", primary: ["u1", "u2"] } });
  });

  it("does not call the server for an invalid configuration", () => {
    const calls = mockApi(() => ({ status: 200, body: [] }));
    renderWithProviders(<RotationPreview values={{ ...values, primary: [] }} />);
    expect(screen.getByText(/at least one person/)).toBeInTheDocument();
    expect(calls.find((c) => c.url === "/schedules/preview")).toBeUndefined();
  });
});

describe("integrations", () => {
  it("validates the webhook URL before sending and sends the CSRF token", async () => {
    const calls = mockApi((url, init) => {
      if (url === "/me") return { status: 200, body: meFixture("admin", ADMIN) };
      if (url === "/integrations" && (!init?.method || init.method === "GET")) return { status: 200, body: [] };
      if (url === "/teams") return { status: 200, body: [] };
      if (url === "/integrations/events") return { status: 200, body: ["rotation_changed", "rotation_approaching"] };
      if (url === "/integrations/discord") return { status: 201, body: {} };
    });
    const user = userEvent.setup();
    renderWithProviders(<IntegrationsPage />);
    await user.click((await screen.findAllByRole("button", { name: "Connect Discord" }))[0]!);
    await user.type(screen.getByLabelText("Name"), "#oncall");
    await user.type(screen.getByLabelText("Webhook URL"), "https://evil.example/hook");
    await user.click(screen.getByRole("button", { name: "Connect" }));
    expect(await screen.findByText(/Use a Discord webhook URL/)).toBeInTheDocument();
    expect(calls.find((c) => c.url === "/integrations/discord")).toBeUndefined();

    await user.clear(screen.getByLabelText("Webhook URL"));
    await user.type(screen.getByLabelText("Webhook URL"), "https://discord.com/api/webhooks/123/abcdefghijklmnopqrstuvwxyz");
    await user.click(screen.getByRole("button", { name: "Connect" }));
    await waitFor(() => expect(calls.find((c) => c.url === "/integrations/discord")).toBeDefined());
    const post = calls.find((c) => c.url === "/integrations/discord")!;
    expect((post.init?.headers as Record<string, string>)["X-CSRF-Token"]).toBe("csrf-123");
  });
});

describe("notification history", () => {
  it("shows failures and retries honestly", async () => {
    mockApi((url) => {
      if (url === "/me") return { status: 200, body: meFixture("admin", ADMIN) };
      if (url === "/integrations") return { status: 200, body: [] };
      if (url.startsWith("/notifications/deliveries"))
        return {
          status: 200,
          body: {
            limit: 50,
            offset: 0,
            has_more: false,
            items: [
              { id: "d1", status: "failed", attempts: 6, max_attempts: 6, next_attempt_at: "2026-01-01T00:00:00Z", last_status_code: 404, last_error: "Discord answered 404", delivered_at: null, created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z", event_type: "rotation_changed", event_scheduled_for: "2026-01-01T00:00:00Z", schedule_id: "s1", schedule_name: "SRE", integration_id: "i1", integration_name: "#ops" },
              { id: "d2", status: "pending", attempts: 2, max_attempts: 6, next_attempt_at: new Date(Date.now() + 120_000).toISOString(), last_status_code: 503, last_error: "Discord answered 503", delivered_at: null, created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z", event_type: "rotation_approaching", event_scheduled_for: "2026-01-01T00:00:00Z", schedule_id: "s1", schedule_name: "SRE", integration_id: "i1", integration_name: "#ops" },
            ],
          },
        };
    });
    renderWithProviders(<NotificationsPage />);
    expect(await screen.findByText("Discord answered 404")).toBeInTheDocument();
    expect(screen.getAllByText("Failed").some((el) => el.tagName === "SPAN")).toBe(true);
    expect(screen.getByText(/Retry in 2 min/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });
});
