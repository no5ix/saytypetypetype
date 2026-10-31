# Repository Guidelines

## Project Structure & Module Organization

SayTypeTypeType contains a Tauri desktop app and an independent website.

- `web/src/`: React/TypeScript desktop settings and Windows overlay; tests in `web/tests/`.
- `src-tauri/src/`: Rust backend and native macOS overlay. Integration tests and fixtures live in `src-tauri/src/tests/`.
- `website/`: Next.js site with independent dependencies. Routes live in `app/`, bilingual documentation in `content/docs/`, assets in `public/`, and tests in `tests/`.
- `assets/`, `scripts/`, `docs/`: desktop resources, packaging utilities, and architecture/testing documentation.

## Build, Test, and Development Commands

Use Node.js 22 and pnpm 10. Install separately with `pnpm install --frozen-lockfile` and `pnpm --dir website install --frozen-lockfile`.

| Command | Purpose |
| --- | --- |
| `pnpm dev` | Run the desktop app with hot reload. |
| `pnpm dev:web` / `pnpm build:web` | Serve/build the desktop frontend. |
| `pnpm lint:ci` | Check Biome, Rust formatting, and Clippy without rewriting files. |
| `pnpm test` | Run Vitest and Rust unit tests. |
| `pnpm --dir website dev` | Start the website locally. |
| `pnpm --dir website lint` / `typecheck` / `test` / `build` | Run website checks; repeat the prefix per command. |

`pnpm format` and `pnpm check` rewrite files; keep resulting changes scoped. Packaging instructions are in `docs/testing-and-release.md`.

## Coding Style & Naming Conventions

Desktop TypeScript uses Biome: two-space indentation, double quotes, semicolons, and 100-column lines. Website code uses ESLint and Prettier: two spaces, no semicolons, and 80-column lines. Rust follows `cargo fmt`. Use PascalCase component names, camelCase functions/hooks, and snake_case Rust modules. Write English comments; remove unused code.

## Testing Guidelines

Use Vitest/jsdom and Testing Library for desktop UI (`*.test.ts`/`*.test.tsx`), inline Rust unit tests, and Node's test runner through `tsx` for website tests. Cover changed logic; no coverage threshold is configured. Run `pnpm test:asr` or `pnpm test:llm` when relevant resources are available. Report skipped integration tests and native checks explicitly; browser tests do not prove native recording behavior.

## Commit & Pull Request Guidelines

Follow English Conventional Commits, such as `fix(hotkey): prevent conflicting recordings`. Keep titles under 72 characters; explain motivation in optional bullets. Describe PR behavior and validation, link issues, and include UI screenshots. Push or publish only when authorized.

## Security & Agent Instructions

Keep API keys, signing credentials, recordings, and local configuration out of commits. Use sanitized example files. Preserve separate desktop/website dependencies and follow `website/AGENTS.md` within that directory. Prefer `trash` over permanent deletion.
