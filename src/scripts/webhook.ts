import 'dotenv/config'
import { createHmac, randomBytes } from 'node:crypto'
import { loadConfig } from '../config.js'

/**
 * Reads and moves the X Account Activity registration.
 *
 *   npm run webhook                                # where X delivers today
 *   npm run webhook -- --set <url>                 # preview the move
 *   npm run webhook -- --set <url> --confirm       # do it
 *   npm run webhook -- --delete <id> --confirm     # retire one by hand
 *
 * X holds the delivery URL in its own configuration, not in this repo, so
 * moving the service to a new hostname is only half a move until this runs.
 *
 * Three things about this API are worth knowing before changing any of it.
 *
 * The app may hold exactly ONE webhook. Registering a second is refused with
 * `WebhookLimitExceeded`, so a move is a delete followed by a create, and there
 * is no moment where both URLs are registered. The account is without a webhook
 * in between; the cron poll is what covers that, and is why this is survivable.
 *
 * The endpoints disagree about credentials, one by one rather than by family.
 * Most want the app-only bearer; creating a subscription acts for the account
 * and wants OAuth 1.0a user context. There is no rule to memorise, so every
 * call goes out with the bearer and retries with user context when X says that
 * is what it wanted — X names the supported type in the 403, which is the only
 * reliable source. Guessing is worse than it sounds: sending user context to an
 * app-only endpoint does not produce an authentication error but a bare
 * `503 Service Unavailable` with no detail, which reads exactly like an outage.
 * That is also why the SDK's own webhook client cannot manage webhooks at all
 * here — it sends user context for all of them — and why these calls are made
 * by hand.
 *
 * X sends the CRC challenge during the create call, so a URL that is not
 * serving is refused there rather than accepted and silently dead. That is the
 * one piece of luck here: the risky half fails before anything is torn down.
 * If it fails anyway, the previous URL is put back before the error is raised.
 */

const config = loadConfig()
const argv = process.argv.slice(2)
const confirm = argv.includes('--confirm')

function flag(name: string): string | undefined {
  const index = argv.indexOf(name)
  return index >= 0 ? argv[index + 1] : undefined
}

const target = flag('--set')
const deleteId = flag('--delete')
const API = 'https://api.x.com'

/** RFC 5849 percent-encoding, which differs from encodeURIComponent on !*'(). */
const enc = (value: string): string =>
  encodeURIComponent(value).replace(
    /[!*'()]/g,
    (ch) => `%${ch.charCodeAt(0).toString(16).toUpperCase()}`,
  )

/**
 * An OAuth 1.0a header for one request. The JSON body is deliberately not part
 * of the signature base: only query parameters are, and X signs these endpoints
 * that way.
 */
function oauth1Header(method: string, url: string): string {
  const parsed = new URL(url)
  const oauth: Record<string, string> = {
    oauth_consumer_key: config.X_API_KEY,
    oauth_nonce: randomBytes(16).toString('hex'),
    oauth_signature_method: 'HMAC-SHA1',
    oauth_timestamp: Math.floor(Date.now() / 1000).toString(),
    oauth_token: config.X_ACCESS_TOKEN,
    oauth_version: '1.0',
  }
  const signed: Record<string, string> = { ...oauth }
  for (const [key, value] of parsed.searchParams) signed[key] = value

  const parameters = Object.keys(signed)
    .sort()
    .map((key) => `${enc(key)}=${enc(signed[key] ?? '')}`)
    .join('&')
  const base = [
    method.toUpperCase(),
    enc(`${parsed.origin}${parsed.pathname}`),
    enc(parameters),
  ].join('&')
  const key = `${enc(config.X_API_SECRET)}&${enc(config.X_ACCESS_TOKEN_SECRET)}`
  const header = { ...oauth, oauth_signature: createHmac('sha1', key).update(base).digest('base64') }

  return `OAuth ${Object.keys(header)
    .sort()
    .map((name) => `${enc(name)}="${enc(header[name as keyof typeof header] ?? '')}"`)
    .join(', ')}`
}

async function attempt(
  method: string,
  path: string,
  auth: 'bearer' | 'oauth1',
  body: unknown,
): Promise<{ ok: boolean; status: number; payload: Record<string, unknown>; detail: string }> {
  const url = `${API}${path}`
  const response = await fetch(url, {
    method,
    headers: {
      authorization:
        auth === 'bearer' ? `Bearer ${config.X_BEARER_TOKEN}` : oauth1Header(method, url),
      ...(body === undefined ? {} : { 'content-type': 'application/json' }),
    },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  })
  const text = await response.text()
  const payload = text ? (JSON.parse(text) as Record<string, unknown>) : {}
  const errors = payload.errors as Array<{ message?: string }> | undefined
  const detail =
    errors?.map((error) => error.message).join('; ') ||
    (payload.detail as string | undefined) ||
    text.slice(0, 200)
  return { ok: response.ok, status: response.status, payload, detail }
}

async function call(
  method: string,
  path: string,
  options: { body?: unknown } = {},
): Promise<Record<string, unknown>> {
  if (!config.X_BEARER_TOKEN) {
    throw new Error('X_BEARER_TOKEN is required to manage the webhook; set it in .env')
  }
  let result = await attempt(method, path, 'bearer', options.body)

  // X names the credential it wanted in the 403 itself. Believe it rather than
  // keeping a table of which endpoint takes which.
  if (!result.ok && result.status === 403 && /OAuth 1\.0a/i.test(result.detail)) {
    result = await attempt(method, path, 'oauth1', options.body)
  }

  if (!result.ok) throw new Error(`${method} ${path} -> ${result.status}: ${result.detail}`)
  return result.payload
}

interface Webhook {
  id?: string
  url?: string
  valid?: boolean
  created_at?: string
}

async function list(): Promise<Webhook[]> {
  const response = await call('GET', '/2/webhooks')
  return (response.data as Webhook[] | undefined) ?? []
}

async function subscriberIds(webhookId: string): Promise<string[]> {
  const response = await call(
    'GET',
    `/2/account_activity/webhooks/${encodeURIComponent(webhookId)}/subscriptions/all/list`,
  )
  const data = response.data as { subscriptions?: Array<{ user_id?: string }> } | undefined
  return (data?.subscriptions ?? []).flatMap((entry) => (entry.user_id ? [entry.user_id] : []))
}

async function describe(hooks: Webhook[]): Promise<void> {
  if (hooks.length === 0) {
    console.log('\n  Nothing is registered. X delivers nothing; only the cron poll finds mentions.')
    return
  }
  for (const hook of hooks) {
    console.log(`\n  ${hook.url}`)
    console.log(`    id ${hook.id}   valid ${hook.valid}   created ${hook.created_at ?? 'unknown'}`)
    if (!hook.id) continue
    try {
      const subscribers = await subscriberIds(hook.id)
      console.log(`    subscriptions: ${subscribers.join(', ') || 'none'}`)
    } catch (error) {
      console.log(`    subscriptions unreadable: ${(error as Error).message}`)
    }
  }
}

async function register(url: string): Promise<string> {
  const created = await call('POST', '/2/webhooks', { body: { url } })
  const id = (created.data as Webhook | undefined)?.id
  if (!id) throw new Error(`X accepted the webhook but returned no id: ${JSON.stringify(created)}`)
  return id
}

async function main(): Promise<void> {
  const existing = await list()

  if (deleteId) {
    if (!confirm) {
      console.log(`Would delete webhook ${deleteId}. Re-run with --confirm.`)
      return
    }
    await call('DELETE', `/2/webhooks/${encodeURIComponent(deleteId)}`)
    console.log(`Deleted ${deleteId}.`)
    return
  }

  if (!target) {
    console.log(`Account: @${config.X_BOT_USERNAME}`)
    await describe(existing)
    console.log('\nMove it with: npm run webhook -- --set <url> --confirm')
    return
  }

  const already = existing.find((hook) => hook.url === target)
  const stale = existing.filter((hook) => hook.url !== target)

  if (!confirm) {
    console.log(`Account: @${config.X_BOT_USERNAME}`)
    console.log(`\nWould register: ${target}`)
    if (already) console.log('  (already registered; it would only be subscribed)')
    for (const hook of stale) {
      console.log(`Would first retire: ${hook.url} (${hook.id})`)
    }
    if (stale.length > 0) {
      console.log('\nThe app holds one webhook, so the old one goes before the new one arrives.')
      console.log('Between the two, only the cron poll finds mentions.')
    }
    console.log('\nRe-run with --confirm.')
    return
  }

  let webhookId = already?.id
  if (webhookId) {
    console.log(`${target} is already registered as ${webhookId}.`)
  } else {
    for (const hook of stale) {
      if (!hook.id) continue
      console.log(`Retiring ${hook.url} (${hook.id}) ...`)
      await call('DELETE', `/2/webhooks/${encodeURIComponent(hook.id)}`)
    }
    console.log(`Registering ${target} ...`)
    try {
      webhookId = await register(target)
    } catch (error) {
      const previous = stale[0]?.url
      if (previous) {
        console.log(`  refused; putting ${previous} back ...`)
        await register(previous).catch(() => {
          console.error(
            `  COULD NOT RESTORE ${previous}. The account has no webhook and only the cron poll\n` +
              `  finds mentions. Re-run: npm run webhook -- --set ${previous} --confirm`,
          )
        })
      }
      throw error
    }
    console.log(`  id ${webhookId}`)
  }

  console.log('Subscribing the account ...')
  try {
    await call(
      'POST',
      `/2/account_activity/webhooks/${encodeURIComponent(webhookId)}/subscriptions/all`,
      { body: {} },
    )
  } catch (error) {
    // A duplicate subscription is an error rather than a no-op here, and that
    // is a success for our purposes. The read-back below is the real check.
    console.log(`  (${(error as Error).message})`)
  }

  const subscribers = await subscriberIds(webhookId)
  if (subscribers.length === 0) {
    throw new Error(
      'the new webhook has no subscription, so X will deliver nothing to it. ' +
        'Subscribe with an access token belonging to the bot account.',
    )
  }
  console.log(`  subscribed: ${subscribers.join(', ')}`)

  console.log('\nDone. X now delivers to:')
  await describe(await list())
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : error)
  process.exit(1)
})
