# Kitelab DB

A simple, native desktop client for MySQL and PostgreSQL — browse tables, edit data in place
and run SQL. Free and open source (MIT).

MySQL болон PostgreSQL-д зориулсан, TablePlus-тэй төстэй боловч илүү энгийн native клиент.

**Website:** [kitedb.dotly.mn](https://kitedb.dotly.mn)

## Platforms

| Platform | Status | Source | Download |
| --- | --- | --- | --- |
| macOS (Apple Silicon, 14+) | ✅ Available | [`macos/`](macos) | [KitelabDB.dmg](https://github.com/kitelab-dev/kitedb/releases/latest/download/KitelabDB.dmg) |
| Linux | 🚧 Planned | — | — |

> The macOS app is not notarized yet. On first launch open **System Settings → Privacy & Security → Open Anyway**,
> or run `xattr -dr com.apple.quarantine "/Applications/Kitelab DB.app"`.

## Repository layout

```
macos/        SwiftUI + AppKit app (build, test and packaging: see macos/README.md)
site/         kitedb.dotly.mn landing page (static)
wrangler.jsonc  Cloudflare Workers config that serves site/ — deployed on every push to main
.github/      release workflow: a v* tag builds the macOS .dmg and attaches it to the GitHub release
```

## License

[MIT](LICENSE). Bundled third-party libraries are listed in
[macos/THIRD_PARTY_NOTICES.md](macos/THIRD_PARTY_NOTICES.md).
