import { useEffect, useState, type FormEvent } from "react";
import { Navigate, useNavigate, useSearchParams } from "react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { api, errorMessage } from "@/lib/api";
import { useMe } from "@/lib/queries";
import { Button, Field, Input } from "@/components/ui";
import { Wordmark } from "@/app/Layout";
import { browserTimezone, formatTime, zoneLabel } from "@/lib/time";

const ERRORS: Record<string, string> = {
  registration_closed: "This server only accepts invited people. Ask an administrator for an invitation, then sign in with the invited address.",
  email_not_verified: "Google has not verified this email address, so it cannot be used to sign in.",
  account_disabled: "This account has been disabled.",
  invalid_state: "The sign-in link expired or was opened in another browser. Please try again.",
  verification_failed: "Google sign-in could not be verified. Please try again.",
  access_denied: "Sign-in was cancelled.",
  not_configured: "Google sign-in is not configured on this server.",
  invalid_request: "The sign-in response was incomplete. Please try again.",
};

function safeReturnTo(value: string | null): string {
  return value && value.startsWith("/") && !value.startsWith("//") ? value : "/";
}

/** The one live element: the current time in the zones on-call teams most often straddle. */
function ZoneClock() {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const t = setInterval(() => setNow(new Date()), 15_000);
    return () => clearInterval(t);
  }, []);
  const zones = Array.from(new Set([browserTimezone(), "UTC"]));
  return (
    <dl className="flex flex-wrap gap-x-10 gap-y-3">
      {zones.map((z) => (
        <div key={z} className="flex flex-col-reverse">
          <dt className="text-base">{z === "UTC" ? "UTC" : z.replace(/_/g, " ")}</dt>
          <dd className="text-4xl font-semibold tracking-[-0.03em] tabular">
            {formatTime(now.toISOString(), z)} <span className="text-xl font-medium">{zoneLabel(z, now)}</span>
          </dd>
        </div>
      ))}
    </dl>
  );
}

export function LoginPage() {
  const [params] = useSearchParams();
  const returnTo = safeReturnTo(params.get("return_to"));
  const error = params.get("error");
  const { data: me } = useMe();
  const providers = useQuery({ queryKey: ["providers"], queryFn: () => api.get<{ google: boolean; dev_login: boolean }>("/auth/providers") });
  const [email, setEmail] = useState("");
  const [devError, setDevError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const qc = useQueryClient();
  const navigate = useNavigate();

  if (me) return <Navigate to={returnTo} replace />;

  async function devLogin(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setDevError(null);
    try {
      await api.post("/auth/dev/login", { email });
      await qc.invalidateQueries({ queryKey: ["me"] });
      navigate(returnTo);
    } catch (err) {
      setDevError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="grid min-h-dvh lg:grid-cols-[1.4fr_1fr]">
      <section className="flex flex-col justify-between gap-10 bg-signal px-6 py-8 text-signal-ink md:gap-16 md:px-12 md:py-12">
        <Wordmark onSignal />
        <div className="rise">
          <h1 className="max-w-[12ch] text-display font-semibold">Your on-call, under control.</h1>
          <p className="mt-6 max-w-xl text-[19px] leading-snug">
            Rotations that hand over on time, in every timezone, with Discord told the moment someone new holds the pager.
          </p>
        </div>
        <div className="max-md:hidden">
          <ZoneClock />
        </div>
      </section>

      <section className="flex flex-col justify-center px-6 py-10 md:px-12 md:py-12">
        <div className="rise rise-2 w-full max-w-sm">
          <h2 className="text-title font-semibold">Sign in</h2>
          {error && (
            <p role="alert" className="mt-5 bg-danger-soft px-4 py-3 text-base">
              {ERRORS[error] ?? "Sign-in failed. Please try again."}
            </p>
          )}
          <div className="flex flex-col gap-4">
            {providers.data?.google !== false && (
              <a
                href={`/api/v1/auth/google/start?return_to=${encodeURIComponent(returnTo)}`}
                aria-disabled={!providers.data?.google}
                className="mt-8 inline-flex h-12 items-center justify-center gap-3 bg-ink px-5 text-[16px] font-medium text-ground transition-[filter] hover:brightness-125 aria-disabled:pointer-events-none aria-disabled:opacity-50"
              >
                <svg viewBox="0 0 24 24" className="size-5" aria-hidden>
                  <path fill="#4285F4" d="M23.5 12.3c0-.8-.1-1.6-.2-2.3H12v4.5h6.5a5.6 5.6 0 0 1-2.4 3.6v3h3.9c2.2-2.1 3.5-5.1 3.5-8.8z" />
                  <path fill="#34A853" d="M12 24c3.2 0 6-1.1 8-2.9l-3.9-3c-1.1.7-2.5 1.2-4.1 1.2-3.1 0-5.8-2.1-6.7-5H1.3v3.1A12 12 0 0 0 12 24z" />
                  <path fill="#FBBC05" d="M5.3 14.3a7.2 7.2 0 0 1 0-4.6V6.6h-4a12 12 0 0 0 0 10.8z" />
                  <path fill="#EA4335" d="M12 4.8c1.8 0 3.3.6 4.6 1.8l3.4-3.4A12 12 0 0 0 1.3 6.6l4 3.1c.9-2.9 3.6-4.9 6.7-4.9z" />
                </svg>
                Continue with Google
              </a>
            )}
            {providers.data && !providers.data.google && !providers.data.dev_login && (
              <p className="mt-8 text-base">No sign-in method is configured on this server. See the setup guide in the documentation.</p>
            )}
          </div>

          {providers.data?.dev_login && (
            <form onSubmit={devLogin} className="mt-8 bg-surface-2 p-5">
              <h3 className="text-lg font-semibold">Development sign-in</h3>
              <p className="mt-1 text-base">Enabled with AUTH_DEV_LOGIN. Anyone can sign in as any address; it is refused in production.</p>
              <div className="mt-4 flex flex-col gap-3">
                <Field label="Email" htmlFor="dev-email" error={devError ?? undefined}>
                  <Input id="dev-email" type="email" required value={email} onChange={(e) => setEmail(e.target.value)} placeholder="you@example.com" />
                </Field>
                <Button type="submit" variant="primary" disabled={busy}>
                  Sign in without Google
                </Button>
              </div>
            </form>
          )}
          <p className="mt-10 text-base text-ink-soft">Sessions are kept in a secure, HttpOnly cookie. Signing in creates a fresh session.</p>
        </div>
      </section>
    </div>
  );
}
