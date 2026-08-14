import SwiftUI

@main
struct LanPulseApp: App {
    @StateObject private var session = LanPulseSession()

    var body: some Scene {
        WindowGroup {
            ContentView(backend: session)
                .ignoresSafeArea()
        }
    }
}
