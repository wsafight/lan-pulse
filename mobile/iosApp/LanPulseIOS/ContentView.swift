import Shared
import SwiftUI

struct ContentView: UIViewControllerRepresentable {
    let backend: LanPulseSession

    func makeUIViewController(context: Context) -> UIViewController {
        IosLanPulseClientKt.MainViewController(backend: backend)
    }

    func updateUIViewController(_ uiViewController: UIViewController, context: Context) {}
}
