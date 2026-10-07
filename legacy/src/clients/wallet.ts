import {
  createPublicClient,
  createWalletClient,
  formatUnits,
  http,
  parseUnits,
  type Address,
} from 'viem'
import { privateKeyToAccount } from 'viem/accounts'
import { robinhood } from 'viem/chains'

/**
 * The account's own wallet on Robinhood Chain, an Arbitrum L2 whose gas is ETH.
 *
 * It holds exactly one capability, an ERC-20 transfer, because a signer is only
 * as safe as its narrowest surface: there is no approve, no swap, no arbitrary
 * call, and no way to move the native balance. The key funds gas and nothing
 * else can reach it.
 */

/** The three functions this needs. A fuller ABI would only widen the surface. */
const ERC20 = [
  {
    type: 'function',
    name: 'transfer',
    stateMutability: 'nonpayable',
    inputs: [
      { name: 'to', type: 'address' },
      { name: 'amount', type: 'uint256' },
    ],
    outputs: [{ name: '', type: 'bool' }],
  },
  {
    type: 'function',
    name: 'balanceOf',
    stateMutability: 'view',
    inputs: [{ name: 'account', type: 'address' }],
    outputs: [{ name: '', type: 'uint256' }],
  },
  {
    type: 'function',
    name: 'decimals',
    stateMutability: 'view',
    inputs: [],
    outputs: [{ name: '', type: 'uint8' }],
  },
] as const

export interface TokenWallet {
  address: Address
  /** Whole tokens the wallet holds. */
  balance(): Promise<number>
  /** Native ETH, which pays for gas. Zero here means nothing can be sent. */
  gasBalance(): Promise<bigint>
  /** Sends whole tokens and waits for the receipt. Throws if it reverts. */
  send(to: Address, amount: number): Promise<{ txHash: string }>
}

export class Erc20Wallet implements TokenWallet {
  readonly address: Address
  private readonly account: ReturnType<typeof privateKeyToAccount>
  private readonly token: Address
  private readonly publicClient: ReturnType<typeof createPublicClient>
  private readonly walletClient: ReturnType<typeof createWalletClient>
  private cachedDecimals?: number

  constructor(options: { privateKey: string; token: string; rpcUrl?: string }) {
    const key = options.privateKey.startsWith('0x')
      ? (options.privateKey as `0x${string}`)
      : (`0x${options.privateKey}` as `0x${string}`)
    this.account = privateKeyToAccount(key)
    this.address = this.account.address
    this.token = options.token as Address
    const transport = http(options.rpcUrl || robinhood.rpcUrls.default.http[0])
    this.publicClient = createPublicClient({ chain: robinhood, transport })
    this.walletClient = createWalletClient({
      account: this.account,
      chain: robinhood,
      transport,
    })
  }

  /** Read once from the contract rather than assuming 18. */
  private async decimals(): Promise<number> {
    this.cachedDecimals ??= Number(
      await this.publicClient.readContract({
        address: this.token,
        abi: ERC20,
        functionName: 'decimals',
      }),
    )
    return this.cachedDecimals
  }

  async balance(): Promise<number> {
    const [raw, decimals] = await Promise.all([
      this.publicClient.readContract({
        address: this.token,
        abi: ERC20,
        functionName: 'balanceOf',
        args: [this.address],
      }),
      this.decimals(),
    ])
    return Number(formatUnits(raw, decimals))
  }

  async gasBalance(): Promise<bigint> {
    return this.publicClient.getBalance({ address: this.address })
  }

  async send(to: Address, amount: number): Promise<{ txHash: string }> {
    const decimals = await this.decimals()
    const value = parseUnits(String(amount), decimals)

    // Simulated first, so a revert -- no balance, a token that refuses the
    // transfer, a wrong contract address -- is an error here rather than a
    // failed transaction the account has already promised in public.
    const { request } = await this.publicClient.simulateContract({
      account: this.account,
      address: this.token,
      abi: ERC20,
      functionName: 'transfer',
      args: [to, value],
    })
    const txHash = await this.walletClient.writeContract(request)

    // Waiting for the receipt is what makes the returned hash a fact. A hash
    // handed back at broadcast time is a hash that may still revert, and the
    // account would have quoted it in a reply either way.
    const receipt = await this.publicClient.waitForTransactionReceipt({
      hash: txHash,
      timeout: 90_000,
    })
    if (receipt.status !== 'success') {
      throw new Error(`the transfer reverted on chain: ${txHash}`)
    }
    return { txHash }
  }
}
