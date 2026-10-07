import { MongoClient, type Collection } from 'mongodb'

/**
 * Who the account has paid, and where.
 *
 * Keyed on the numeric X id rather than the handle, because a handle is a
 * mutable label -- this account renamed itself once already and every check
 * keyed on the old name went quiet. The handles are kept as history so a person
 * is still recognisable in the ledger after they rename, but nothing is ever
 * decided by them.
 *
 * Holding the address alongside the person is what makes one abuse cheap to
 * refuse: twenty fresh accounts pointing at one wallet is the ordinary shape of
 * airdrop farming, and it is invisible if you only ever ask "has THIS account
 * been paid before".
 */
export interface Recipient {
  userId: string
  /** Most recent first. A rename appends rather than overwrites. */
  usernames: string[]
  /** Every address this person has been paid at. */
  addresses: string[]
  firstSeenAt: string
  lastAirdropAt?: string
}

export interface RecipientRegistry {
  /** Who, if anyone, has already been paid at this address. */
  ownerOfAddress(address: string): Promise<Recipient | undefined>
  get(userId: string): Promise<Recipient | undefined>
  /** Records the pairing. Safe to call again with the same values. */
  remember(entry: { userId: string; username?: string; address: string }): Promise<void>
  close(): Promise<void>
}

export class MongoRecipientRegistry implements RecipientRegistry {
  private readonly client: MongoClient
  private readonly recipients: Collection<Recipient>
  private indexed?: Promise<void>

  constructor(options: { uri: string; database: string; collection?: string }) {
    this.client = new MongoClient(options.uri)
    this.recipients = this.client
      .db(options.database)
      .collection<Recipient>(options.collection ?? 'recipients')
  }

  private async ready(): Promise<void> {
    this.indexed ??= (async () => {
      await this.client.connect()
      await this.recipients.createIndex({ userId: 1 }, { unique: true })
      // Addresses are stored lower-cased, so this lookup is exact rather than
      // defeated by somebody re-pasting the same wallet in a different case.
      await this.recipients.createIndex({ addresses: 1 })
    })()
    await this.indexed
  }

  async ownerOfAddress(address: string): Promise<Recipient | undefined> {
    await this.ready()
    return (await this.recipients.findOne({ addresses: address.toLowerCase() })) ?? undefined
  }

  async get(userId: string): Promise<Recipient | undefined> {
    await this.ready()
    return (await this.recipients.findOne({ userId })) ?? undefined
  }

  async remember(entry: {
    userId: string
    username?: string
    address: string
  }): Promise<void> {
    await this.ready()
    const now = new Date().toISOString()
    await this.recipients.updateOne(
      { userId: entry.userId },
      {
        $setOnInsert: { userId: entry.userId, firstSeenAt: now },
        $set: { lastAirdropAt: now },
        // addToSet, so a person paid twice at one address has one entry for it,
        // and a rename adds a name without losing the one they had before.
        $addToSet: {
          addresses: entry.address.toLowerCase(),
          ...(entry.username ? { usernames: entry.username } : {}),
        },
      },
      { upsert: true },
    )
  }

  async close(): Promise<void> {
    await this.client.close()
  }
}
