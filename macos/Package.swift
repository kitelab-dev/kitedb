// swift-tools-version:6.0
import Foundation
import PackageDescription

// Homebrew prefixes for the native client libraries. Override with
// LIBPQ_PREFIX / MYSQL_PREFIX when they live somewhere else.
let env = ProcessInfo.processInfo.environment
let libpq = env["LIBPQ_PREFIX"] ?? "/opt/homebrew/opt/libpq"
let mysql = env["MYSQL_PREFIX"] ?? "/opt/homebrew/opt/mysql"

let package = Package(
    name: "KiteDB",
    platforms: [.macOS(.v14)],
    products: [
        .executable(name: "KiteDB", targets: ["KiteDB"]),
    ],
    targets: [
        .systemLibrary(name: "CLibPQ", path: "Sources/CLibPQ"),
        .systemLibrary(name: "CMySQL", path: "Sources/CMySQL"),
        .executableTarget(
            name: "KiteDB",
            dependencies: ["CLibPQ", "CMySQL"],
            swiftSettings: [
                .swiftLanguageMode(.v5),
                .unsafeFlags([
                    "-Xcc", "-I\(libpq)/include",
                    "-Xcc", "-I\(mysql)/include/mysql",
                ]),
            ],
            linkerSettings: [
                .unsafeFlags([
                    "-L\(libpq)/lib",
                    "-L\(mysql)/lib",
                    "-Xlinker", "-rpath", "-Xlinker", "@executable_path/../Frameworks",
                ]),
            ]
        ),
        .testTarget(
            name: "KiteDBTests",
            dependencies: ["KiteDB"],
            swiftSettings: [.swiftLanguageMode(.v5)]
        ),
    ]
)
