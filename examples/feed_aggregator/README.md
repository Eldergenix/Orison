## Feed aggregator example

Demonstrates outbound HTTP + a background queue. Periodically fetches feeds,
parses them into a typed model, and enqueues new entries for processing by a
worker.

```bash
ori check --json examples/feed_aggregator/src/main.ori
ori capability --policy "http,net.outbound,db.read,db.write" --json \
  examples/feed_aggregator/src/api.ori
ori openapi --json examples/feed_aggregator/src/api.ori
```
