# Plug in your accounts

Back to the [tour](../../README.md). Status Quo can show **any number** of Claude, Codex and GitHub sign-ins at once, for example a personal and a work account side by side.

## The idea: account keys

An **account key** names one sign-in:

| Key | Meaning |
|---|---|
| `claude`, `codex`, `github` | your default accounts, the ones the tools use without any setting |
| `claude:work`, `codex:lab`, `github:team` | extra accounts: the kind, a colon, the short name you chose |

Every number Status Quo stores (a usage reading, a token record, a session, an activity item, a health entry) carries its account key, so two accounts can never mix. A widget picks exactly one key, or `both` for the sessions and memory widgets.

## Default accounts

Nothing to configure. Sign in with the tool itself and Status Quo reads what it saved:

```sh
claude          # then /login inside it
codex login
gh auth login
```

## Add an extra account

An extra account is another *home folder* that the same tool keeps its sign-in in. Create the sign-in with the tool first, pointing it at the new folder:

| Tool | Create the second sign-in |
|---|---|
| Claude | `CLAUDE_CONFIG_DIR=~/.claude-work claude` then `/login` |
| Codex | `CODEX_HOME=~/.codex-work codex login` |
| GitHub | `GH_CONFIG_DIR=~/.config/gh-work gh auth login` |

Then tell Status Quo:

1. **Customize → More accounts → + Add account.**
2. Choose the **kind** (`claude`, `codex` or `github`) and a short **name**: lower-case letters, digits, `-` and `_`, unique within its kind. The key becomes `kind:name`.
3. Press **Browse…** next to *Home folder* (for GitHub: *gh config folder*) and click your way to the folder. The window starts at the folder already typed, opens folders when you click them, and **Use this folder** stores the choice. A folder inside your home is stored as `~/…`, so a shared setting works for another person.
4. GitHub only: list the repositories this account reads, one `owner/name` per line.
5. **Save changes.**

Nothing secret is stored in Status Quo's settings, only where the tool keeps its own folder. Up to 16 extra accounts are accepted, and every folder path must be absolute or start with `~/`.

## Point a widget at an account

Two ways, same result:

* In **+ Add widget**, the menu lists one line per account: `claude / Account usage`, `claude:work / Account usage`, `codex / Token history`, …
* For an existing widget, open **Customize → Widgets**, open the entry and change its **provider** drop-down.

A widget set to a key that does not exist (you removed the account) says **unknown**. It never shows another account's numbers.

## How often accounts are read

Account usage is read without calling any model, and each account has its own retry delay, so one account's failure does not slow the others. The interval is the slider *Account polling interval* in Customize (300 to 3600 seconds, default 300). If a read fails or is rate-limited, the last good value stays on screen with its age and the reason.

## Limits worth knowing

* The live session states that come from hooks describe the **default** accounts only; transcripts of extra accounts still produce their sessions, tokens and progress.
* Resuming a session of an extra account opens the tool without that account's home folder.
* Progress counts come from each session's own task list, whatever the account.

## Where the folders are

Defaults are `~/.claude`, `~/.codex`, and your workflow-memory notes folder (Customize → Sources, each with **Browse…**). On Windows the same names live under your user folder.
