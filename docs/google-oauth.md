# Setting up Google sign-in

1. Open the [Google Cloud console](https://console.cloud.google.com/), choose or create a project.
2. **APIs & Services → OAuth consent screen.** Choose *Internal* if everyone uses your Google Workspace, otherwise *External*. Fill in the app name and support email. The scopes OncallKit asks for are `openid`, `email` and `profile`.
3. **APIs & Services → Credentials → Create credentials → OAuth client ID.**
   - Application type: *Web application*.
   - Authorised redirect URI: `<PUBLIC_URL>/api/v1/auth/google/callback`, for example
     - `http://localhost:8080/api/v1/auth/google/callback` with Docker Compose locally,
     - `http://localhost:5173/api/v1/auth/google/callback` with the Vite dev server,
     - `https://oncall.example.com/api/v1/auth/google/callback` in production.
4. Copy the client ID and secret into `.env` as `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET`, and restart the API.

`PUBLIC_URL` must match the address in the browser exactly (scheme, host and port), or the OAuth state cookie will not be sent back and sign-in will fail with "expired or opened in another browser".

The client secret is used only by the API server. It never reaches the browser.

## Who can sign in

| `REGISTRATION_MODE` | New accounts allowed for |
| --- | --- |
| `open` | any Google account with a verified email |
| `invite_only` | addresses with a pending invitation, and bootstrapped administrators |
| `domains` | verified emails in `ALLOWED_EMAIL_DOMAINS`, plus invited addresses |

Existing accounts can always sign in (unless disabled). Accounts are identified by Google's stable subject id, so a changed email address still signs in to the same account.
