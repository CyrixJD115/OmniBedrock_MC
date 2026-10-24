FROM node:22-bookworm-slim AS build
WORKDIR /app
# Split deployments serve the SvelteKit node server from nginx; the single
# binary build uses the default static adapter instead.
ENV ADAPTER=node
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/ ./
# Development config is locally customizable; the tracked template supplies the build plugin.
RUN cp vite.config.example.ts vite.config.ts && npm run build

FROM node:22-bookworm-slim AS runtime
WORKDIR /app
COPY --from=build /app/build ./build
COPY --from=build /app/package.json /app/package-lock.json ./
RUN npm ci --omit=dev && npm cache clean --force
ENV NODE_ENV=production HOST=0.0.0.0 PORT=17755
USER node
EXPOSE 17755
CMD ["node", "build"]
