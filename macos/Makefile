export DEVELOPER_DIR ?= /Applications/Xcode.app/Contents/Developer

.PHONY: app dmg run test icon clean

app:
	./scripts/build-app.sh

dmg: app
	./scripts/make-dmg.sh

run: app
	open "build/KiteDB.app"

# Integration tests; set KITELAB_MYSQL / KITELAB_PG=host:port:user:password
test:
	swift test

icon:
	swift scripts/make-icon.swift Resources

clean:
	rm -rf .build build
