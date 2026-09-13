import 'dotenv/config'
import { XApiClient } from '../clients/x.js'
import { GiphyClient } from '../clients/giphy.js'
import { loadConfig } from '../config.js'
import { AgentReplyModel } from '../agent/reply-model.js'
import { applyHouseStyle } from '../agent/house-style.js'
import { loadPersona } from '../agent/soul.js'
import { ReplyHarness } from '../harness/reply-harness.js'
import { CortexMemory } from '../memory/cortex.js'
import { GlobalMarketClient } from '../market/global.js'
import { TokenMarket } from '../market/market.js'
import type { XPost } from '../types.js'
import { buildMedia } from '../media/wiring.js'

/**
 * Shows what the bot would reply, without replying.
 *
 *   npm run dry-run -- <conversation-or-post-id>   # every reply in that thread
 *   npm run dry-run                                 # the current mention window
 *   npm run dry-run -- --text "..." [--from name]   # a hypothetical post
 *
 * The last form is how tone is tested without waiting for someone to actually
 * be rude on the internet.
 *
 * Nothing is published. Memory is still written, because the draft the bot
 * would really produce depends on the same capture-then-recall the cron does.
 */

const config = loadConfig()
const argv = process.argv.slice(2)

function flag(name: string): string | undefined {
  const index = argv.indexOf(name)
  return index >= 0 ? argv[index + 1] : undefined
}

const hypothetical = flag('--text')
// Who the hypothetical post is from. The numeric id matters: the follow check
// and every per-person rule key on it, so a made-up id makes the rehearsal
// answer a different question from the real one.
const asUserId = flag('--as-id')
const target = argv[0]?.startsWith('--') ? undefined : argv[0]

const memory = new CortexMemory(
  { apiUrl: config.CORTEX_API_URL, actor: config.CORTEX_ACTOR, bearer: config.CORTEX_TOKEN },
  config.CORTEX_SCOPE,
)
const x = new XApiClient({
  username: config.X_BOT_USERNAME,
  apiKey: config.X_API_KEY,
  apiSecret: config.X_API_SECRET,
  accessToken: config.X_ACCESS_TOKEN,
  accessTokenSecret: config.X_ACCESS_TOKEN_SECRET,
  ...(config.X_BEARER_TOKEN ? { bearerToken: config.X_BEARER_TOKEN } : {}),
})
const gifs = config.GIPHY_API_KEY
  ? {
      source: new GiphyClient({ apiKey: config.GIPHY_API_KEY, channel: config.GIPHY_CHANNEL }),
      channel: config.GIPHY_CHANNEL,
    }
  : undefined

const media = buildMedia(config)
const harness = new ReplyHarness(
  memory,
  new AgentReplyModel({
    memory,
    x,
    ...(media ? { media } : {}),
    botUsername: config.X_BOT_USERNAME,
    dryRun: true,
    llm: {
      baseUrl: config.LLM_BASE_URL,
      apiKey: config.LLM_API_KEY,
      model: config.LLM_MODEL,
    },
    maxSteps: config.AGENT_MAX_STEPS,
    persona: loadPersona({
      soulPath: config.SOUL_PATH,
      contextPath: config.CONTEXT_PATH,
      skillsDir: config.SKILLS_DIR,
    }),
    searchLimit: config.X_SEARCH_MAX_RESULTS,
    // The rehearsal gets the same market access the real drafter has, or it is
    // rehearsing a reply the cron would not produce.
    ...(config.CHART_TOKEN_ADDRESS
      ? {
          market: {
            market: new TokenMarket({
              network: config.CHART_CHAIN,
              ...(config.SOLANA_RPC_URL ? { solanaRpcUrl: config.SOLANA_RPC_URL } : {}),
              ...(config.GMGN_BASE_URL ? { gmgnBaseUrl: config.GMGN_BASE_URL } : {}),
              watchlist: config.KOL_WALLETS,
            }),
            address: config.CHART_TOKEN_ADDRESS,
            symbol: config.CHART_TOKEN_SYMBOL,
            global: new GlobalMarketClient(),
          },
        }
      : {}),
    ...(config.EXA_API_KEY ? { exaApiKey: config.EXA_API_KEY } : {}),
    ...(gifs ? { gifs } : {}),
    ownTokenAddresses: config.OWN_TOKEN_ADDRESSES,
  }),
  // The dry run has to see the same thread the cron would, or it is testing a
  // different reply from the one that would go out.
  config.THREAD_CONTEXT_MAX_POSTS > 0
    ? { x, maxPosts: config.THREAD_CONTEXT_MAX_POSTS }
    : undefined,
  media?.reader,
)

const botUsername = config.X_BOT_USERNAME.toLowerCase()
const posts: XPost[] = hypothetical
  ? [
      {
        id: flag('--post-id') ?? '0',
        text: hypothetical,
        authorId: asUserId ?? '0',
        authorUsername: flag('--from') ?? 'hypothetical_user',
      },
    ]
  : target
  ? (await x.getConversation(target, 100)).filter(
      (post) => post.authorUsername?.toLowerCase() !== botUsername,
    )
  : await x.findMentions({ maxResults: config.MAX_REPLIES_PER_RUN })

console.log(`\n${posts.length} post(s) to evaluate. Nothing will be published.\n`)

for (const post of posts) {
  const author = post.authorUsername ? `@${post.authorUsername}` : (post.authorId ?? 'unknown')
  console.log('─'.repeat(72))
  console.log(`FROM ${author}  (${post.id}, ${post.createdAt ?? 'no timestamp'})`)
  console.log(`  ${post.text.replace(/\n/g, '\n  ')}`)
  try {
    const draft = await harness.generate(post)
    const styled = applyHouseStyle(draft.text, {
      maxEmoji: config.MAX_EMOJI_PER_POST,
      ...(post.authorUsername ? { replyingTo: post.authorUsername } : {}),
    })
    console.log(`\nWOULD REPLY (${styled.text.length} chars):`)
    console.log(`  ${styled.text.replace(/\n/g, '\n  ')}`)
    if (draft.gifQuery) console.log(`  [gif: ${draft.gifQuery}]`)
    console.log(
      `  [tone: ${draft.incomingTone ?? 'unjudged'}` +
        `${draft.incomingTone === 'negative' ? ', NOT stored in memory' : ''}]`,
    )
    if (styled.fixes.length > 0) console.log(`  [house style: ${styled.fixes.join('; ')}]`)
  } catch (error) {
    console.log(`\nFAILED: ${(error as Error).message}`)
  }
  console.log()
}
