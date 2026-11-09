## Provider property writes do not advance the modified tick (planned)

GNU advances `buffer-modified-tick` even under `with-silent-modifications`:

```
silent-property-tick-delta=1
```

Provider-origin writes will not advance it. Redisplay must not invalidate
unrelated tick-keyed caches. Authored writes retain ordinary mutation semantics.
The storage checkpoint has one origin and does not yet implement this rule.
