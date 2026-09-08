import 'dotenv/config'
import { createServer } from 'node:http'
import { createInterface } from 'node:readline/promises'
import { readFileSync, writeFileSync } from 'node:fs'
import { OAuth1 } from '@xdevplatform/xdk'

/**
 * Runs the X OAuth 1.0a three-legged flow once and writes the resulting user
 * access token pair into .env. The bot itself cannot start until this has run,
 * because it signs every write with that pair.
 *
 *   npm run auth:x
 */

const ENV_PATH = new URL('../../.env', import.meta.url).pathname

function required(name: string): string {
  const value = process.env[name]
  if (!value) throw new Error(`${name} is not set in .env`)
  return value
}

/** Rewrites a key in .env in place, leaving comments and order untouched. */
function writeEnvValue(contents: string, key: string, value: string): string {
  const line = `${key}=${value}`
  const pattern = new RegExp(`^${key}=.*$`, 'm')
  return pattern.test(contents) ? contents.replace(pattern, line) : `${contents}\n${line}\n`
}

async function verifierFromPin(): Promise<string> {
  const readline = createInterface({ input: process.stdin, output: process.stdout })
  try {
    return (await readline.question('Paste the PIN shown by X: ')).trim()
  } finally {
    readline.close()
  }
}

/** Serves the registered callback URL until X redirects the browser back to it. */
async function verifierFromCallback(
  callbackUrl: string,
  expectedToken: string,
): Promise<string> {
  const url = new URL(callbackUrl)
  const port = Number(url.port || (url.protocol === 'https:' ? 443 : 80))

  return new Promise((resolve, reject) => {
    const server = createServer((request, response) => {
      const incoming = new URL(request.url ?? '/', `http://${request.headers.host}`)
      if (incoming.pathname !== url.pathname) {
        response.writeHead(404).end('not found')
        return
      }

      const token = incoming.searchParams.get('oauth_token')
      const verifier = incoming.searchParams.get('oauth_verifier')
      const denied = incoming.searchParams.get('denied')

      if (denied) {
        response.writeHead(200, { 'content-type': 'text/plain' }).end('Authorization denied.')
        server.close()
        reject(new Error('the authorization request was denied on X'))
        return
      }
      if (!verifier || token !== expectedToken) {
        response.writeHead(400, { 'content-type': 'text/plain' }).end('Unexpected callback.')
        server.close()
        reject(new Error('X returned a callback for a different request token'))
        return
      }

      response
        .writeHead(200, { 'content-type': 'text/plain' })
        .end('Authorized. You can close this tab and return to the terminal.')
      server.close()
      resolve(verifier)
    })

    server.on('error', reject)
    server.listen(port, () => {
      console.log(`Listening for the X redirect on ${url.origin}${url.pathname}`)
    })
  })
}

const callback = process.env.X_OAUTH_CALLBACK_URL?.trim() || 'oob'
const oauth = new OAuth1({
  apiKey: required('X_API_KEY'),
  apiSecret: required('X_API_SECRET'),
  callback,
})

const requestToken = await oauth.getRequestToken()
console.log('\nOpen this URL and authorize the app as the bot account:\n')
console.log(`  ${oauth.getAuthorizationUrl()}\n`)

const verifier =
  callback === 'oob'
    ? await verifierFromPin()
    : await verifierFromCallback(callback, requestToken.oauthToken)

const accessToken = await oauth.getAccessToken(verifier)

let contents = readFileSync(ENV_PATH, 'utf8')
contents = writeEnvValue(contents, 'X_ACCESS_TOKEN', accessToken.accessToken)
contents = writeEnvValue(contents, 'X_ACCESS_TOKEN_SECRET', accessToken.accessTokenSecret)
writeFileSync(ENV_PATH, contents, { mode: 0o600 })

console.log('\nWrote X_ACCESS_TOKEN and X_ACCESS_TOKEN_SECRET to .env.')
console.log('DRY_RUN is still true, so nothing will publish until you change it.')
