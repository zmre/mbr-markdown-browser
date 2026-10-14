import SwiftUI

@main
struct MBRMobileApp: App {
    @State private var model = AppModel()
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(self.model)
                .environment(self.model.store)
                .onOpenURL { self.model.openDocument($0) }
                .task {
                    // Spawn the WebContent process while the vault list is up.
                    self.model.pool.prewarm()
                    await SelfTest.startIfRequested(self.model)
                }
        }
        .onChange(of: self.scenePhase) { _, phase in
            switch phase {
            case .background: self.model.didEnterBackground()
            case .active: self.model.didBecomeActive()
            default: break
            }
        }
    }
}
