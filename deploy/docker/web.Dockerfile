# syntax=docker/dockerfile:1.7
# OncallKit web app: build static assets with Node, serve them with unprivileged nginx, which also
# proxies /api to the API so the browser sees a single origin.
FROM node:22-alpine AS build
WORKDIR /app
COPY apps/web/package.json apps/web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY apps/web ./
RUN npm run build

FROM nginxinc/nginx-unprivileged:1.27-alpine
# Where /api is proxied to. Compose names the service "api"; the Helm chart sets its own.
ENV API_UPSTREAM=http://api:8080
COPY deploy/docker/nginx.conf.template /etc/nginx/templates/default.conf.template
COPY --from=build /app/dist /usr/share/nginx/html
EXPOSE 8080
