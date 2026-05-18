# Project rules

## Git history: linear only

- **Never create merge commits** in this repo.
- **Always rebase** when integrating branches; keep history linear.
- When updating a branch against `main`: `git fetch && git rebase origin/main`, not `git merge`.
- When landing a PR: rebase-and-merge (or squash-and-merge), never the "Create a merge commit" button.
- If a rebase produces conflicts, resolve them — do not fall back to a merge as a shortcut.
