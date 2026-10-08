# Small image for engine tests: node (MCP server, egress proxy) and git.
FROM docker.io/library/node:22-alpine
COPY nucleus-build-ca.crt /tmp/nucleus-build-ca.crt
RUN cp /etc/ssl/certs/ca-certificates.crt /tmp/ca-orig.crt \
 && if [ -s /tmp/nucleus-build-ca.crt ]; then cat /tmp/nucleus-build-ca.crt >> /etc/ssl/certs/ca-certificates.crt; fi \
 && apk add --no-cache git \
 && mv /tmp/ca-orig.crt /etc/ssl/certs/ca-certificates.crt \
 && rm -f /tmp/nucleus-build-ca.crt
