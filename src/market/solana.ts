import type { HolderAccount, HolderSnapshot } from './types.js'

interface RpcResponse<T> {
  result?: T
  error?: { code?: number; message?: string }
}

interface SupplyResult {
  value?: { amount?: string; decimals?: number; uiAmount?: number }
}

interface LargestResult {
  value?: Array<{ address?: string; uiAmount?: number | null }>
}

/**
 * Holder concentration, read straight off a Solana RPC.
 *
 * Deliberately not a paid indexer: `getTokenLargestAccounts` is a standard RPC
 * method every provider implements, so pointing `SOLANA_RPC_URL` at a private
 * node is the only change needed when the public one rate-limits -- which it
 * does, often, and which is why every caller of this treats a failure as
 * "unknown" rather than as an outage.
 *
 * One caveat the analysis has to respect: these are token *accounts*, not
 * owners. A pool's vault is the largest account of almost every token, and one
 * person can hold three. Read it as concentration, never as "the top holder".
 */
export class SolanaRpc {
  constructor(
    private readonly options: { url?: string; timeoutMs?: number } = {},
  ) {}

  private async call<T>(method: string, params: unknown[]): Promise<T> {
    const controller = new AbortController()
    const timer = setTimeout(() => controller.abort(), this.options.timeoutMs ?? 15_000)
    try {
      const response = await fetch(this.options.url ?? 'https://api.mainnet-beta.solana.com', {
        method: 'POST',
        signal: controller.signal,
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
      })
      if (!response.ok) throw new Error(`Solana RPC ${method} answered ${response.status}`)
      const body = (await response.json()) as RpcResponse<T>
      if (body.error) throw new Error(`Solana RPC ${method}: ${body.error.message ?? 'failed'}`)
      if (body.result === undefined) throw new Error(`Solana RPC ${method} returned no result`)
      return body.result
    } finally {
      clearTimeout(timer)
    }
  }

  async holders(mint: string): Promise<HolderSnapshot> {
    const [supply, largest] = await Promise.all([
      this.call<SupplyResult>('getTokenSupply', [mint]),
      this.call<LargestResult>('getTokenLargestAccounts', [mint]),
    ])

    const total = supply.value?.uiAmount ?? 0
    if (!total) throw new Error(`Solana RPC returned no supply for ${mint}`)

    const top: HolderAccount[] = (largest.value ?? []).flatMap((account) => {
      const amount = account.uiAmount
      if (!account.address || typeof amount !== 'number') return []
      return [{ address: account.address, amount, share: amount / total }]
    })

    return {
      totalSupply: total,
      decimals: supply.value?.decimals ?? 0,
      top,
      top10Share: top.slice(0, 10).reduce((sum, account) => sum + account.share, 0),
      source: 'solana-rpc',
    }
  }
}
