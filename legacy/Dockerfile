# syntax=docker/dockerfile:1

# The X bot: a Fastify webhook server plus the reply cron, in one process.
#
# The TypeScript service now lives in legacy/ while the Rust rewrite takes the
# repo root. The build context is still the repo root, because the persona and
# content files stay there: `docker build -f legacy/Dockerfile .`

FROM node:22-alpine AS builder
WORKDIR /app
COPY legacy/package.json legacy/package-lock.json ./
RUN npm ci
COPY legacy/tsconfig.json ./
COPY legacy/src ./src
RUN npm run build

FROM node:22-alpine AS runner
WORKDIR /app
ENV NODE_ENV=production

# Fonts, for the chart renderer. resvg draws text through the system fonts and
# alpine ships none: without this the chart still renders and every label --
# price axis, ticker, timestamps -- silently disappears, which looks like a
# broken chart rather than a missing package.
RUN apk add --no-cache font-noto

COPY legacy/package.json legacy/package-lock.json ./
RUN npm ci --omit=dev && npm cache clean --force
COPY --from=builder /app/dist ./dist

# The persona is read at startup from the working directory, not compiled in.
COPY SOUL.md ./SOUL.md
COPY CONTEXT.md ./CONTEXT.md
COPY skills ./skills
# The serial's bible and voice: the article's rooms read them at boot.
COPY narrative ./narrative
COPY assets ./assets
COPY calendar.json ./calendar.json

# The reply job keeps its cursor on disk, and the chart agent writes each PNG it
# renders next to it, so the path is a volume in compose.
RUN mkdir -p /app/data/charts /app/data/moon && addgroup --system --gid 1001 nodejs \
  && adduser --system --uid 1001 bot && chown -R bot:nodejs /app/data
USER bot

EXPOSE 3001
CMD ["node", "dist/index.js"]
