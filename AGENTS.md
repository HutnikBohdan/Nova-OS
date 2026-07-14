# Nova OS repository rules

## Git synchronization

- After every coherent change block passes its relevant tests, commit and push
  the current branch to GitHub.
- Keep `master` stable. Develop substantial changes on `codex/<topic>` branches
  and expose them through a draft pull request until acceptance gates pass.
- Never leave verified source changes only on the local machine.
- Do not commit build caches, temporary VM disks or logs, downloaded third-party
  tools, credentials, or access tokens.
- Publish verified Nova BIOS/UEFI images and Nova VM Lab binaries as private
  GitHub pre-release assets tied to the exact source commit that produced them.

## Verification

- A successful host build is not release proof. Record QEMU serial evidence and
  exercise the relevant Guardian, hardware, recovery, or application gate.
- Do not merge a feature to `master` until its documented acceptance criteria
  pass. Keep incomplete work explicit in the roadmap and release notes.
