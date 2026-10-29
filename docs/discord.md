# Discord notifications

## Create a webhook

1. In Discord, open the channel's settings (**Edit Channel**), then **Integrations → Webhooks → New Webhook**.
2. Name it (for example "OncallKit") and click **Copy Webhook URL**.
3. In OncallKit, go to **Integrations → Connect Discord**, paste the URL, choose the events and save.
4. Click **Send test**. OncallKit reports exactly what Discord answered.

Only `https://discord.com/api/webhooks/<id>/<token>` style URLs (also `discordapp.com`, `ptb.` and `canary.`, an API version segment and `?thread_id=`) are accepted. The URL is encrypted at rest, never shown again, and never logged; the integration page shows only the webhook id.

## Events

| Event | When |
| --- | --- |
| Rotation hand-over | The rotation hands over to someone new (primary and/or secondary) |
| New primary / secondary on call | Anyone new takes the layer, including through an override |
| Reminder before a shift | At each configured offset before a person's shift (per schedule, e.g. 24 h and 1 h) |
| Schedule created / updated | A schedule is created or changed |
| Override created / canceled | Cover is added or removed |
| Another channel failed to deliver | A delivery to a different channel failed after all retries |

A channel scoped to a team only receives events for that team's schedules.

## Example message

> **DevOps On-call: On-call handover**
> **Primary** Andi · **Secondary** Budi · **Previous primary** Riky
> **Effective from** 3 November 2026, 09:00 WIB
> **Next rotation** 10 November 2026, 09:00 WIB
> **Team** DevOps · **Schedule** DevOps On-call

The title links to the schedule at your `PUBLIC_URL`.

## Mentions

With **Mention people** enabled, the person going on call is @mentioned if they have added their Discord user id to their OncallKit profile. Mentions are restricted to those user ids: names and prefixes cannot trigger `@everyone`, `@here` or role pings.

## Reliability

Deliveries are stored before sending, retried on 429, 5xx and network errors with exponential backoff and jitter (respecting `Retry-After`), and recorded attempt by attempt in **Notification history**. A webhook that Discord rejects permanently (for example deleted, 404) fails immediately. Failed deliveries can be retried from the history page.

A successful delivery means Discord accepted the message, not that anyone read it.
