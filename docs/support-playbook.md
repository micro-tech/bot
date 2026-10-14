# Support playbook — John's greatest hits

The support-mail watcher (SYSADMIN-01) feeds this file to the drafting
sub-agent so replies sound like **you**, not a generic bot. It's your input:
write it in your own words, one `##` section per problem you see often.

**Location:** `<helix-dir>/support_playbook.md` (next to `config.toml` on the R630).
**Optional:** if the file is absent, drafting still works — the draft just
notes "no playbook entry matched".

## Format

```markdown
# Support playbook — John's greatest hits

## Laptop won't power on
Symptoms: no lights, no fan, completely dead.
John's fix: reseat the RAM, try a known-good charger, check the DC jack for looseness. If the board's dead, quote a replacement.

## Windows PC running slow
Symptoms: takes 10 minutes to boot, fans screaming.
John's fix: check Task Manager for a runaway process, SSD upgrade if it's still on a spinner, blow out the dust.

## <next problem customers describe>
Symptoms: ...
John's fix: ...
```

- Section titles should use the words customers actually say ("won't
  power on", not "power delivery subsystem failure") — the matcher keys
  off title words.
- Keep each section short; the drafter sees at most 3 sections, ~1500
  chars each.
- The draft records which sections were cited, so you can see the
  provenance when reviewing.
