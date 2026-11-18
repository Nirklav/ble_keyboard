# Compat breaks may be intentional

Backward compatibility between the app and older keyboard firmware usually matters. Some changes break it on purpose, though. On 2026-10-30, the app started sending 16 firmware chunks at a time with no fallback for older firmware. A code-review `--fix` added a `Versions::features` flag to keep older firmware working, and the user had it reverted.

**Why:** The user decides compatibility case by case. Whether a break was intended can't be seen from the code.

**How to apply:** Still report a lost compatibility as a finding, but don't fix it on your own (no flags or fallbacks). Leave it to the user and ask whether the break is intended.
