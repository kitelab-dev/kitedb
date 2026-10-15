import SwiftUI

enum DatabaseKind: String, Codable, CaseIterable, Identifiable {
    case mysql
    case postgres

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .mysql: "MySQL"
        case .postgres: "PostgreSQL"
        }
    }

    var shortName: String {
        switch self {
        case .mysql: "MySQL"
        case .postgres: "PG"
        }
    }

    var defaultPort: Int {
        switch self {
        case .mysql: 3306
        case .postgres: 5432
        }
    }

    var defaultUser: String {
        switch self {
        case .mysql: "root"
        case .postgres: "postgres"
        }
    }

    var badgeColor: Color {
        switch self {
        case .mysql: Color(red: 0.0, green: 0.46, blue: 0.56)
        case .postgres: Color(red: 0.2, green: 0.4, blue: 0.75)
        }
    }
}

enum SSLMode: String, Codable, CaseIterable, Identifiable {
    case disable
    case prefer
    case require

    var id: String { rawValue }

    var displayName: String {
        switch self {
        case .disable: "Disabled"
        case .prefer: "Preferred"
        case .require: "Required"
        }
    }
}

enum ConnectionColor: String, Codable, CaseIterable, Identifiable {
    case none, gray, red, orange, yellow, green, blue, purple

    var id: String { rawValue }

    var color: Color? {
        switch self {
        case .none: nil
        case .gray: .gray
        case .red: .red
        case .orange: .orange
        case .yellow: .yellow
        case .green: .green
        case .blue: .blue
        case .purple: .purple
        }
    }
}

struct ConnectionConfig: Codable, Identifiable, Hashable {
    var id = UUID()
    var name = ""
    var kind: DatabaseKind = .mysql
    var host = "127.0.0.1"
    var port = 3306
    var user = "root"
    var database = ""
    var sslMode: SSLMode = .prefer
    var color: ConnectionColor = .none

    var displayName: String { name.isEmpty ? host : name }

    var summary: String {
        var text = "\(user)@\(host):\(port)"
        if !database.isEmpty { text += "/\(database)" }
        return text
    }
}
