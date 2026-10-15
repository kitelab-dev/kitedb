import SwiftUI

struct ConnectionFormView: View {
    @Environment(\.dismiss) private var dismiss
    @State private var config: ConnectionConfig
    @State private var password: String
    @State private var testState: TestState = .idle
    let isNew: Bool
    let onSave: (ConnectionConfig, String, Bool) -> Void

    enum TestState: Equatable {
        case idle, testing
        case success(String)
        case failure(String)
    }

    init(config: ConnectionConfig, isNew: Bool, onSave: @escaping (ConnectionConfig, String, Bool) -> Void) {
        _config = State(initialValue: config)
        _password = State(initialValue: isNew ? "" : Keychain.password(for: config.id))
        self.isNew = isNew
        self.onSave = onSave
    }

    var body: some View {
        VStack(spacing: 0) {
            Form {
                Section {
                    Picker("Type", selection: $config.kind) {
                        ForEach(DatabaseKind.allCases) { Text($0.displayName).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .onChange(of: config.kind) { old, new in
                        if config.port == old.defaultPort { config.port = new.defaultPort }
                        if config.user == old.defaultUser { config.user = new.defaultUser }
                    }
                    TextField("Name", text: $config.name, prompt: Text("My database"))
                    LabeledContent("Color") {
                        HStack(spacing: 8) {
                            ForEach(ConnectionColor.allCases) { tag in
                                colorSwatch(tag)
                            }
                        }
                    }
                }
                Section {
                    TextField("Host", text: $config.host, prompt: Text("127.0.0.1"))
                    TextField("Port", value: $config.port, format: .number.grouping(.never))
                    TextField("User", text: $config.user)
                    SecureField("Password", text: $password)
                    TextField("Database", text: $config.database, prompt: Text(config.kind == .postgres ? "postgres" : "optional"))
                    Picker("SSL", selection: $config.sslMode) {
                        ForEach(SSLMode.allCases) { Text($0.displayName).tag($0) }
                    }
                }
            }
            .formStyle(.grouped)

            Divider()
            HStack {
                Button("Test") { Task { await test() } }
                    .disabled(testState == .testing)
                testStatus
                Spacer()
                Button("Cancel", role: .cancel) { dismiss() }
                    .keyboardShortcut(.cancelAction)
                Button("Save") { save(connect: false) }
                Button("Connect") { save(connect: true) }
                    .keyboardShortcut(.defaultAction)
            }
            .padding(16)
        }
        .frame(width: 480)
        .fixedSize(horizontal: false, vertical: true)
    }

    private func colorSwatch(_ tag: ConnectionColor) -> some View {
        Button {
            config.color = tag
        } label: {
            ZStack {
                if let color = tag.color {
                    Circle().fill(color)
                } else {
                    Circle().strokeBorder(.secondary, lineWidth: 1)
                    Image(systemName: "slash.circle").foregroundStyle(.secondary).font(.system(size: 14))
                }
                if config.color == tag {
                    Circle().strokeBorder(Color.primary, lineWidth: 2).padding(-3)
                }
            }
            .frame(width: 16, height: 16)
        }
        .buttonStyle(.plain)
        .help(tag.rawValue.capitalized)
    }

    @ViewBuilder
    private var testStatus: some View {
        switch testState {
        case .idle:
            EmptyView()
        case .testing:
            ProgressView().controlSize(.small)
        case let .success(version):
            Label("Connected · \(version)", systemImage: "checkmark.circle.fill")
                .foregroundStyle(.green)
                .lineLimit(1)
        case let .failure(message):
            Label(message, systemImage: "xmark.octagon.fill")
                .foregroundStyle(.red)
                .lineLimit(2)
                .font(.caption)
                .help(message)
                .textSelection(.enabled)
        }
    }

    private func test() async {
        testState = .testing
        let session = DBSession(driver: DriverFactory.make(config: config, password: password))
        do {
            let version = try await session.run { driver in
                try driver.connect()
                defer { driver.close() }
                return driver.serverVersion
            }
            testState = .success(version)
        } catch {
            testState = .failure(error.localizedDescription)
        }
    }

    private func save(connect: Bool) {
        var config = self.config
        config.host = config.host.trimmingCharacters(in: .whitespaces)
        if config.host.isEmpty { config.host = "127.0.0.1" }
        onSave(config, password, connect)
    }
}
