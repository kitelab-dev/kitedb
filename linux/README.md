# KiteDB — Linux

macOS хувилбартай ижил боломжтой, Rust хэл дээр бичигдсэн Linux native MySQL / PostgreSQL клиент.
UI нь GTK 4 + libadwaita (`ColumnView` grid, `TextView` editor), серверүүдтэй цэвэр Rust драйвераар
(`mysql`, `postgres` crate, TLS нь rustls) холбогдоно — libpq, libmysqlclient, OpenSSL шаардахгүй.

## Боломжууд

- **Холболтын жагсаалт** — MySQL / MariaDB / PostgreSQL, өнгөт шошго, SSL горим, "Test" товч.
  Нууц үг системийн keyring-д (Secret Service: GNOME Keyring, KWallet, KeePassXC…) хадгалагдана;
  keyring байхгүй үед зөвхөн тухайн хэрэглэгч уншиж чадах `~/.config/kitedb/passwords.json` (0600) руу шилжинэ.
  Бусад тохиргоо `~/.config/kitedb/connections.json`.
- **Sidebar** — table / view жагсаалт, хайлт, database солих (header), PG schema сонгох.
  Right-click: шинэ tab-д нээх, SELECT query үүсгэх, нэр хуулах, Truncate / Drop (баталгаажуулалттай).
- **Tab-ууд** — table болон query tab; table дээр дарахад одоогийн table tab-ыг солино (preview tab шиг).
- **Data grid** — 300 мөрөөр хуудаслах, багана дээр дарж эрэмбэлэх, шүүлтүүр
  (багана + оператор + утга, эсвэл raw `WHERE`), CSV экспорт.
- **Засварлах** — double-click / Enter-ээр нүд засах, Tab-аар дараагийн нүд, Esc цуцлах,
  Set NULL, мөр нэмэх (Ctrl+I), устгах (Delete). Өөрчлөлтүүд шар/ногоон/улаанаар тэмдэглэгдэж,
  **Ctrl+S** дарахад нэг transaction-д хадгалагдана (алдаа гарвал rollback). Primary key шаардлагатай.
- **Structure** — баганууд (type, nullable, default, key, extra, comment) ба индексүүд.
- **SQL editor** — syntax highlight, auto-indent, Ctrl+Space / Esc-ээр keyword/table нэр гүйцээх,
  Ctrl+Enter ажиллуулах (сонгосон хэсэг байвал зөвхөн түүнийг), олон statement, Ctrl+. цуцлах.

## Товчлол

| Товч | Үйлдэл |
| --- | --- |
| Ctrl+Shift+K | Холболтын цонх |
| Ctrl+N | Шинэ холболт (холболтын цонхонд) |
| Ctrl+T | Шинэ query tab |
| Ctrl+Enter / Ctrl+. | Query ажиллуулах / цуцлах |
| Ctrl+S | Өөрчлөлт хадгалах |
| Ctrl+I | Мөр нэмэх |
| Ctrl+R, F5 | Refresh |
| Ctrl+PageDown / Ctrl+PageUp | Дараагийн / өмнөх tab |
| Ctrl+W | Tab хаах |
| Ctrl+Q | Гарах |

## Build

Бүх команд `linux/` хавтас дотроос ажиллана. Шаардлага: Rust (stable, 1.88+), GTK 4.14+ ба libadwaita 1.5+ хөгжүүлэлтийн багц.

```sh
sudo apt install libgtk-4-dev libadwaita-1-dev     # Debian / Ubuntu 24.04+
sudo dnf install gtk4-devel libadwaita-devel        # Fedora
sudo pacman -S gtk4 libadwaita                      # Arch

make run          # dev build хийгээд нээнэ
make install      # release build, ~/.local руу суулгана (PREFIX=/usr/local sudo make install)
make package      # build/KiteDB-linux-<arch>.tar.gz (binary + .desktop + icon + install.sh)
make lint         # cargo fmt --check + clippy
```

`v*` tag push хийхэд repo-ийн root дахь `.github/workflows/release.yml` нь macOS-ийн дараа Ubuntu 24.04 runner дээр
`make package`-тэй ижил архив үүсгэж, GitHub Release-д хавсаргана. Ubuntu 24.04-ийн GTK 4.14 / libadwaita 1.5 дээр
build хийдэг тул тэр үеийн болон түүнээс шинэ distro (Fedora 40, Debian 13, Arch…) дээр ажиллана.

Тест (амьд сервер шаардлагатай; тохируулаагүй бол алгасна):

```sh
KITELAB_MYSQL=127.0.0.1:3306:root: KITELAB_PG=127.0.0.1:5432:postgres:secret make test
```

`KITEDB_NO_KEYRING=1` тохируулбал нууц үгийг keyring-д бичихгүй, зөвхөн `passwords.json` ашиглана
(тест, headless орчинд хэрэгтэй).

## Бүтэц

```
linux/
  src/
    db/                    Driver trait, Session (тусдаа thread), MySqlDriver, PostgresDriver, rustls
    models/                холболт (JSON + keyring), table/query tab-ын state, хадгалах transaction
    ui/
      mod.rs               Application, цэс, товчлол
      welcome.rs           холболтын цонх, connection_form.rs
      workspace.rs         холболтын цонх: sidebar, tab-ууд, header
      grid.rs              ColumnView дээрх засварлах боломжтой grid
      editor.rs            SQL editor
      table_tab.rs, query_tab.rs, sidebar.rs, widgets.rs
  tests/                   амьд сервертэй integration тест
  data/                    .desktop, AppStream metainfo, icon
  scripts/                 install.sh, uninstall.sh, package.sh
```
