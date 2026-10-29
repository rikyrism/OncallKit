import type { ReactElement } from "react";
import { render } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { vi } from "vitest";
import { ToastProvider } from "@/components/ui";

type Handler = (url: string, init?: RequestInit) => { status: number; body?: unknown } | undefined;

/** Replace fetch with a router over `/api/v1/...` paths. Unknown paths return 404. */
export function mockApi(handler: Handler) {
  const calls: { url: string; init?: RequestInit }[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input).replace(/^\/api\/v1/, "");
      calls.push({ url, init });
      const res = handler(url, init) ?? { status: 404, body: { error: { code: "not_found", message: "not found" } } };
      return new Response(res.body === undefined ? null : JSON.stringify(res.body), { status: res.status, headers: { "Content-Type": "application/json" } });
    }),
  );
  return calls;
}

export function renderWithProviders(ui: ReactElement, { route = "/" }: { route?: string } = {}) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <MemoryRouter initialEntries={[route]}>
        <ToastProvider>{ui}</ToastProvider>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

export const meFixture = (role: "owner" | "admin" | "scheduler" | "member", permissions: string[]) => ({
  user: {
    user_id: "u1",
    session_id: "s1",
    email: "riky@acme.test",
    display_name: "Riky",
    avatar_url: null,
    discord_user_id: null,
    is_instance_admin: false,
    active_organization_id: "o1",
  },
  csrf_token: "csrf-123",
  memberships: [{ organization_id: "o1", name: "Acme", slug: "acme", role }],
  active_organization_id: "o1",
  role,
  permissions,
  can_create_organization: true,
});
