FROM node:24-bookworm-slim
WORKDIR /app
COPY sdk/package.json ./sdk/package.json
COPY sdk/src ./sdk/src
COPY apps/github-bot/package.json ./apps/github-bot/package.json
COPY apps/github-bot/src ./apps/github-bot/src
RUN chmod -R a+rX /app && mkdir -p /data && chown node:node /data
ENV NODE_ENV=production BOT_DATABASE=/data/github-bot.sqlite
USER node
EXPOSE 3001
HEALTHCHECK --interval=30s --timeout=5s CMD node -e "fetch('http://127.0.0.1:'+(process.env.PORT||3001)+'/health').then(r=>{if(!r.ok)process.exit(1)}).catch(()=>process.exit(1))"
CMD ["node", "--experimental-strip-types", "apps/github-bot/src/main.ts"]
