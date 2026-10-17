## Download

**[KiteDB.dmg](https://github.com/kitelab-dev/kitedb/releases/latest/download/KiteDB.dmg)** — Apple Silicon (M1 or newer), macOS 14 Sonoma or later.

**[KiteDB-linux-x86_64.tar.gz](https://github.com/kitelab-dev/kitedb/releases/latest/download/KiteDB-linux-x86_64.tar.gz)** — Linux x86_64 with GTK 4.14+ and libadwaita 1.5+ (Ubuntu 24.04, Fedora 40, Debian 13, Arch or newer).

## Install

1. Open the `.dmg` and drag **KiteDB** into **Applications**.
2. The app is not notarized by Apple yet, so the first launch is blocked. Either:
   - open **System Settings → Privacy & Security** and click **Open Anyway** next to “KiteDB”, or
   - run `xattr -dr com.apple.quarantine "/Applications/KiteDB.app"` in Terminal.

### Linux

```sh
tar -xzf KiteDB-linux-x86_64.tar.gz
./KiteDB-linux-x86_64/install.sh     # installs to ~/.local; use PREFIX=/usr/local with sudo for all users
```

## Суулгах

1. `.dmg` файлыг нээгээд **KiteDB**-г **Applications** руу чирнэ.
2. Апп Apple-аар notarize хийгдээгүй тул анх нээхэд блоклогдоно. **System Settings → Privacy & Security** хэсэгт **Open Anyway** дарна, эсвэл Terminal дээр `xattr -dr com.apple.quarantine "/Applications/KiteDB.app"` ажиллуулна.

### Linux

Архивыг задлаад `./KiteDB-linux-x86_64/install.sh` ажиллуулна (`~/.local` руу суулгана). GTK 4.14+ болон libadwaita 1.5+ шаардлагатай.
