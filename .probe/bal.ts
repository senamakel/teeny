import { createPublicClient, http, formatUnits } from 'viem'
import { robinhood } from 'viem/chains'
const TOKEN = '0xb9CE619b168f325b4eb8C2E8E073501838C7A407' as const
const WALLET = '0x4C3799279dDF44d97a46dd14927789f23bf47589' as const
const ABI = [{ type:'function', name:'balanceOf', stateMutability:'view', inputs:[{type:'address'}], outputs:[{type:'uint256'}] }] as const
const c = createPublicClient({ chain: robinhood, transport: http() })
const [tiny, eth] = await Promise.all([
  c.readContract({ address: TOKEN, abi: ABI, functionName: 'balanceOf', args: [WALLET] }),
  c.getBalance({ address: WALLET }),
])
console.log('TINY:', formatUnits(tiny as bigint, 18))
console.log('ETH :', formatUnits(eth, 18))
