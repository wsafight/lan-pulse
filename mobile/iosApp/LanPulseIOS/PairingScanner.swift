import AVFoundation
import UIKit
import VisionKit

@MainActor
final class PairingScanner: NSObject, DataScannerViewControllerDelegate {
    private let onCode: (String) -> Void
    private let onFailure: (Error) -> Void
    private let onCancel: () -> Void
    private var controller: DataScannerViewController?

    init(
        onCode: @escaping (String) -> Void,
        onFailure: @escaping (Error) -> Void,
        onCancel: @escaping () -> Void
    ) {
        self.onCode = onCode
        self.onFailure = onFailure
        self.onCancel = onCancel
    }

    func start() {
        guard DataScannerViewController.isSupported, DataScannerViewController.isAvailable else {
            onFailure(LanPulseError.cameraUnavailable)
            return
        }

        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized:
            presentScanner()
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
                Task { @MainActor in
                    guard let self else { return }
                    granted ? self.presentScanner() : self.onFailure(LanPulseError.cameraPermissionRequired)
                }
            }
        default:
            onFailure(LanPulseError.cameraPermissionRequired)
        }
    }

    func dataScanner(
        _ dataScanner: DataScannerViewController,
        didAdd addedItems: [RecognizedItem],
        allItems: [RecognizedItem]
    ) {
        for item in addedItems {
            guard case .barcode(let barcode) = item,
                  let value = barcode.payloadStringValue
            else { continue }
            finish {
                self.onCode(value)
            }
            return
        }
    }

    @objc private func cancel() {
        finish(onCancel)
    }

    private func presentScanner() {
        guard let presenter = Self.topViewController() else {
            onFailure(LanPulseError.cameraUnavailable)
            return
        }
        let scanner = DataScannerViewController(
            recognizedDataTypes: [.barcode(symbologies: [.qr])],
            qualityLevel: .balanced,
            recognizesMultipleItems: false,
            isHighFrameRateTrackingEnabled: false,
            isPinchToZoomEnabled: true,
            isGuidanceEnabled: true,
            isHighlightingEnabled: true
        )
        scanner.delegate = self
        scanner.navigationItem.rightBarButtonItem = UIBarButtonItem(
            barButtonSystemItem: .cancel,
            target: self,
            action: #selector(cancel)
        )
        let navigationController = UINavigationController(rootViewController: scanner)
        navigationController.modalPresentationStyle = .fullScreen
        controller = scanner
        presenter.present(navigationController, animated: true) { [weak self, weak scanner] in
            do {
                try scanner?.startScanning()
            } catch {
                self?.finish { self?.onFailure(error) }
            }
        }
    }

    private func finish(_ completion: @escaping () -> Void) {
        guard let controller else {
            completion()
            return
        }
        controller.stopScanning()
        self.controller = nil
        controller.dismiss(animated: true, completion: completion)
    }

    private static func topViewController() -> UIViewController? {
        let root = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .flatMap(\.windows)
            .first(where: \.isKeyWindow)?
            .rootViewController
        var current = root
        while let presented = current?.presentedViewController {
            current = presented
        }
        return current
    }
}
