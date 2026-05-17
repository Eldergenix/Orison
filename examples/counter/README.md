## Counter example

Smallest possible Orison app: a single counter view with increment / decrement
actions. Exercises the bootstrap UI manifest extractor's per-view tracking.

```bash
ori check --json examples/counter/src/main.ori
ori ui --json examples/counter/src/main.ori
ori run examples/counter/src/main.ori
```
