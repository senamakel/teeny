# syntax=docker/dockerfile:1

# The X bot: a Fastify webhook server plus the reply cron, in one process.

FROM node:22-alpine AS builder
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY . .
RUN npm run build

FROM node:22-alpine AS runner
WORKDIR /app
ENV NODE_ENV=production

# Fonts, for the chart renderer. resvg draws text through the system fonts and
# alpine ships none: without this the chart still renders and every label --
# price axis, ticker, timestamps -- silently disappears, which looks like a
# broken chart rather than a missing package.
RUN apk add --no-cache font-noto

COPY package.json package-lock.json ./
RUN npm ci --omit=dev && npm cache clean --force
COPY --from=builder /app/dist ./dist

# The persona is read at startup from the working directory, not compiled in.
COPY --from=builder /app/SOUL.md ./SOUL.md
COPY --from=builder /app/CONTEXT.md ./CONTEXT.md
COPY --from=builder /app/skills ./skills
# The serial's bible and voice: the article's rooms read them at boot.
COPY --from=builder /app/narrative ./narrative
COPY --from=builder /app/assets ./assets
COPY --from=builder /app/calendar.json ./calendar.json

# The reply job keeps its cursor on disk, and the chart agent writes each PNG it
# renders next to it, so the path is a volume in compose.
RUN mkdir -p /app/data/charts /app/data/moon && addgroup --system --gid 1001 nodejs \
  && adduser --system --uid 1001 bot && chown -R bot:nodejs /app/data
USER bot

EXPOSE 3001
CMD ["node", "dist/index.js"]
