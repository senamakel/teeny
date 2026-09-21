import type { Memory, XTools } from '../types.js'
import { markdownToContentState, type ArticleImageRef } from './content-state.js'
import type { ArticleRecord, ArticleStore } from './store.js'

/**
 * A saved article, published as it is. The rehearsal wrote the record and
 * the pictures; this reads them back, uploads, sends, and marks the record
 * published. Nothing is rewritten or redrawn, so what was reviewed is what
 * goes out.
 */
export async function publishSaved(
  slug: string,
  deps: {
    articles: ArticleStore
    x: Pick<XTools, 'uploadMedia' | 'publishArticle'>
    memory?: Memory
    now?: () => number
    logger?: { info: (object: unknown, message?: string) => void }
  },
): Promise<ArticleRecord> {
  const record = await deps.articles.get(slug)
  if (!record) throw new Error(`no article saved as ${slug}`)
  if (record.status === 'published') throw new Error(`${slug} was already published as post ${record.postId}`)
  if (!record.markdown.trim() || !record.title) throw new Error(`${slug} has no text to publish`)

  const upload = async (file: string): Promise<string> => {
    const media = await deps.articles.media(slug, file)
    if (!media) throw new Error(`${slug} is missing its picture ${file}`)
    const mimeType = media.mimeType as 'image/png' | 'image/jpeg' | 'image/webp'
    return (await deps.x.uploadMedia({ data: media.data, mimeType })).mediaId
  }
  const coverMediaId = record.cover ? await upload(record.cover.file) : undefined
  const refs: Record<string, ArticleImageRef> = {}
  const images = []
  for (const image of record.images) {
    const mediaId = await upload(image.file)
    refs[image.key] = { mediaId, caption: image.caption }
    images.push({ ...image, mediaId })
  }
  const contentState = markdownToContentState(record.markdown, refs)
  let draft: ArticleRecord = {
    ...record,
    status: 'draft',
    ...(record.cover && coverMediaId ? { cover: { ...record.cover, mediaId: coverMediaId } } : {}),
    images,
    contentState,
  }
  await deps.articles.save(draft)
  try {
    const { articleId, postId } = await deps.x.publishArticle({ title: record.title, contentState, ...(coverMediaId ? { coverMediaId } : {}) })
    const publishedAt = new Date((deps.now ?? Date.now)()).toISOString()
    draft = { ...draft, status: 'published', xArticleId: articleId, postId, publishedAt }
    await deps.articles.save(draft)
    deps.logger?.info({ slug, articleId, postId }, 'article: published')
    await deps.memory
      ?.captureActivity('article', postId, {
        slug,
        title: record.title,
        dek: record.dek,
        engine: record.engine,
        premise: record.thought?.premise,
        sections: record.sections.map((section) => section.heading),
        published: true,
      })
      .catch(() => undefined)
    return draft
  } catch (error) {
    await deps.articles.save({ ...draft, status: 'failed', error: error instanceof Error ? error.message : String(error) })
    throw error
  }
}
