# KiteDB — macOS

TablePlus-тэй төстэй, гэхдээ илүү энгийн macOS native MySQL / PostgreSQL клиент.
SwiftUI + AppKit (NSTableView grid, NSTextView editor) дээр бичигдсэн бөгөөд
серверүүдтэй TablePlus-ийн адил албан ёсны `libmysqlclient`, `libpq` C сангуудаар холбогдоно.

## Боломжууд

- **Холболтын жагсаалт** — MySQL / PostgreSQL, өнгөт шошго, SSL горим, "Test" товч.
  Нууц үг macOS Keychain-д хадгалагдана; бусад тохиргоо
  `~/Library/Application Support/KitelabDB/connections.json`.
- **Sidebar** — table / view жагсаалт, хайлт, database солих (toolbar), PG schema сонгох.
  Right-click: шинэ tab-д нээх, SELECT query үүсгэх, нэр хуулах, Truncate / Drop (баталгаажуулалттай).
- **Tab-ууд** — table болон query tab; table дээр дарахад одоогийн table tab-ыг солино (preview tab шиг).
- **Data grid** — 300 мөрөөр хуудаслах, багана дээр дарж эрэмбэлэх, шүүлтүүр
  (багана + оператор + утга, эсвэл raw `WHERE`), CSV экспорт.
- **Засварлах** — double-click / Return-оор нүд засах, Tab-аар дараагийн нүд, Esc цуцлах,
  Set NULL, мөр нэмэх (⌘I), устгах (⌫). Өөрчлөлтүүд шар/ногоон/улаанаар тэмдэглэгдэж,
  **⌘S** дарахад нэг transaction-д хадгалагдана (алдаа гарвал rollback). Primary key шаардлагатай.
- **Structure** — баганууд (type, nullable, default, key, extra, comment) ба индексүүд.
- **SQL editor** — syntax highlight, auto-indent, Esc-ээр keyword/table нэр гүйцээх,
  ⌘↩ ажиллуулах (сонгосон хэсэг байвал зөвхөн түүнийг), олон statement, ⌘. цуцлах.

## Товчлол

| Товч | Үйлдэл |
| --- | --- |
| ⌘⇧K | Холболтын цонх |
| ⌘T | Шинэ query tab |
| ⌘↩ / ⌘. | Query ажиллуулах / цуцлах |
| ⌘S | Өөрчлөлт хадгалах |
| ⌘I | Мөр нэмэх |
| ⌘R | Refresh |
| ⌘⇧] / ⌘⇧[ | Дараагийн / өмнөх tab |
| ⌥⌘W | Tab хаах |

## Build

Бүх команд `macos/` хавтас дотроос ажиллана. Шаардлага: Xcode (SwiftUI macro-ууд Command Line Tools-д байхгүй), Homebrew-ийн
`libpq` болон `mysql` (эсвэл `mysql-client`).

```sh
brew install libpq mysql     # аль хэдийн суусан бол алгас
make app                     # build/KiteDB.app үүсгэнэ
make run                     # build хийгээд нээнэ
make dmg                     # build/KiteDB.dmg
```

`v*` tag push хийхэд repo-ийн root дахь `.github/workflows/release.yml` нь macOS 14 runner дээр build хийж,
`.dmg`-г GitHub Release-д хавсаргана (macOS 14 bottle ашигладаг тул апп macOS 14+ дээр ажиллана;
локал build нь таны macOS-ийн Homebrew сангуудаас хамаарч илүү өндөр хувилбар шаардаж болно).

`scripts/build-app.sh` нь release binary-г `.app` болгож, libpq / libmysqlclient болон
тэдгээрийн хамаарлуудыг (OpenSSL, krb5, zstd, zlib) `Contents/Frameworks` руу хуулж,
ad-hoc sign хийнэ — апп Homebrew-гүйгээр ажиллана. Өөр байршилд суусан бол
`LIBPQ_PREFIX`, `MYSQL_PREFIX` орчны хувьсагчаар зааж өгнө.

Тест (амьд сервер шаардлагатай; тохируулаагүй бол алгасна):

```sh
KITELAB_MYSQL=127.0.0.1:3306:root: KITELAB_PG=127.0.0.1:5432:postgres:secret make test
```

## Бүтэц

```
macos/
  Sources/
    CLibPQ, CMySQL/        C сангуудын module map
    KiteDB/
      App/                 @main, цэс, товчлол
      Database/            DatabaseDriver protocol, MySQLDriver, PostgresDriver, DBSession (serial queue)
      Models/              холболт (JSON + Keychain), workspace, table/query tab-ын state
      Grid/DataGrid.swift  NSTableView дээрх засварлах боломжтой grid
      Editor/SQLEditor.swift
      Views/               SwiftUI дэлгэцүүд
  scripts/                 build-app.sh, make-dmg.sh, make-icon.swift
```
