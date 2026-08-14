import AVFoundation
import Accelerate
import Darwin
import Foundation

struct ReceiverStats: Sendable {
    let packetsReceived: UInt64
    let packetsLost: UInt64
    let bufferMs: Int
}

final class RTPAudioReceiver: @unchecked Sendable {
    let localPort: UInt16

    private let descriptor: Int32
    private let queue = DispatchQueue(label: "com.lanpulse.ios.rtp", qos: .userInteractive)
    private let queueKey = DispatchSpecificKey<UInt8>()
    private var engine = AVAudioEngine()
    private var player = AVAudioPlayerNode()
    private var readSource: DispatchSourceRead?
    private var timeoutTimer: DispatchSourceTimer?
    private var notificationTokens: [NSObjectProtocol] = []
    private var audio: AudioConfig?
    private var format: AVAudioFormat?
    private var jitterBuffer = RTPJitterBuffer()
    private var jitterController: AdaptiveJitterController?
    private var packetPool: [RTPPacket] = []
    private var discardDatagram: [UInt8] = []
    private var audioBufferPool: [AVAudioPCMBuffer] = []
    private var scheduledBufferCount = 0
    private var playbackGeneration: UInt64 = 0
    private var packetsReceived: UInt64 = 0
    private var packetsLost: UInt64 = 0
    private var lastPacketAt: UInt64 = 0
    private var startedPlayout = false
    private var stopped = false
    private var reportedTimeout = false
    private var interrupted = false
    private var onStats: (@Sendable (ReceiverStats) -> Void)?
    private var onFailure: (@Sendable (Error) -> Void)?

    init() throws {
        queue.setSpecific(key: queueKey, value: 1)
        let socketDescriptor = Darwin.socket(AF_INET, SOCK_DGRAM, IPPROTO_UDP)
        guard socketDescriptor >= 0 else { throw LanPulseError.desktopUnavailable }

        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = 0
        address.sin_addr = in_addr(s_addr: INADDR_ANY)
        let bindResult = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { socketAddress in
                Darwin.bind(socketDescriptor, socketAddress, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard bindResult == 0 else {
            Darwin.close(socketDescriptor)
            throw LanPulseError.desktopUnavailable
        }

        var localAddress = sockaddr_in()
        var localLength = socklen_t(MemoryLayout<sockaddr_in>.size)
        let nameResult = withUnsafeMutablePointer(to: &localAddress) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { socketAddress in
                Darwin.getsockname(socketDescriptor, socketAddress, &localLength)
            }
        }
        guard nameResult == 0 else {
            Darwin.close(socketDescriptor)
            throw LanPulseError.desktopUnavailable
        }
        var receiveBufferBytes: Int32 = 256 * 1024
        _ = setsockopt(
            socketDescriptor,
            SOL_SOCKET,
            SO_RCVBUF,
            &receiveBufferBytes,
            socklen_t(MemoryLayout.size(ofValue: receiveBufferBytes))
        )
        descriptor = socketDescriptor
        localPort = UInt16(bigEndian: localAddress.sin_port)
    }

    func start(
        audio: AudioConfig,
        onStats: @escaping @Sendable (ReceiverStats) -> Void,
        onFailure: @escaping @Sendable (Error) -> Void
    ) throws {
        try audio.validate()
        self.audio = audio
        let jitterController = AdaptiveJitterController(
            sampleRate: audio.sampleRate,
            packetMs: audio.packetMs
        )
        self.jitterController = jitterController
        jitterBuffer = RTPJitterBuffer(
            targetPacketCount: jitterController.targetPacketCount,
            maximumPacketCount: max(4, (80 + audio.packetMs - 1) / audio.packetMs)
        )
        let datagramCapacity = audio.payloadBytes + 512
        packetPool = (0..<(jitterBuffer.maximumPacketCount * 2 + 8)).map { _ in
            RTPPacket(capacity: datagramCapacity)
        }
        discardDatagram = [UInt8](repeating: 0, count: datagramCapacity)
        self.onStats = onStats
        self.onFailure = onFailure
        self.lastPacketAt = DispatchTime.now().uptimeNanoseconds

        try configureAudioSession(audio: audio)

        guard let format = AVAudioFormat(
            standardFormatWithSampleRate: Double(audio.sampleRate),
            channels: AVAudioChannelCount(audio.channels)
        ) else { throw LanPulseError.unsupportedAudio }
        self.format = format
        try rebuildAudioGraph()
        observeAudioSession()

        let readSource = DispatchSource.makeReadSource(fileDescriptor: descriptor, queue: queue)
        readSource.setEventHandler { [weak self] in self?.receiveAvailableDatagrams() }
        self.readSource = readSource
        readSource.resume()

        let timer = DispatchSource.makeTimerSource(queue: queue)
        timer.schedule(
            deadline: .now() + .milliseconds(250),
            repeating: .milliseconds(250),
            leeway: .milliseconds(50)
        )
        timer.setEventHandler { [weak self] in self?.checkStreamTimeout() }
        timeoutTimer = timer
        timer.resume()
    }

    func stop() {
        let cleanup = { [self] in
            guard !stopped else { return }
            stopped = true
            readSource?.cancel()
            timeoutTimer?.cancel()
            readSource = nil
            timeoutTimer = nil
            notificationTokens.forEach(NotificationCenter.default.removeObserver)
            notificationTokens.removeAll()
            playbackGeneration &+= 1
            player.stop()
            engine.stop()
            scheduledBufferCount = 0
            audioBufferPool.removeAll()
            jitterBuffer.reset()
            recycleDiscardedPackets()
            packetPool.removeAll()
            discardDatagram.removeAll()
            Darwin.close(descriptor)
            try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
        }
        if DispatchQueue.getSpecific(key: queueKey) != nil {
            cleanup()
        } else {
            queue.sync(execute: cleanup)
        }
    }

    private func receiveAvailableDatagrams() {
        guard let audio, !stopped else { return }
        while true {
            guard let packet = packetPool.popLast() else {
                let count = discardDatagram.withUnsafeMutableBytes { bytes in
                    Darwin.recv(descriptor, bytes.baseAddress, bytes.count, MSG_DONTWAIT)
                }
                guard count > 0 else { return }
                continue
            }
            let count = packet.withUnsafeMutableDatagramBytes { bytes in
                Darwin.recv(descriptor, bytes.baseAddress, bytes.count, MSG_DONTWAIT)
            }
            guard count > 0 else {
                packetPool.append(packet)
                return
            }
            guard packet.parse(
                length: count,
                payloadType: audio.payloadType,
                ssrc: audio.ssrc,
                payloadBytes: audio.payloadBytes
            ) else {
                packetPool.append(packet)
                continue
            }
            let arrivalNanos = DispatchTime.now().uptimeNanoseconds
            lastPacketAt = arrivalNanos
            reportedTimeout = false
            jitterController?.observe(timestamp: packet.timestamp, arrivalNanos: arrivalNanos)
            if let target = jitterController?.targetPacketCount {
                jitterBuffer.updateTarget(target)
            }
            jitterBuffer.insert(packet)
            recycleDiscardedPackets()
            startPlayoutIfReady()
            growScheduledQueueFromBufferedPackets()
        }
    }

    private func checkStreamTimeout() {
        guard !stopped, !interrupted else { return }
        let now = DispatchTime.now().uptimeNanoseconds
        if !reportedTimeout && now &- lastPacketAt > 3_000_000_000 {
            reportedTimeout = true
            onFailure?(LanPulseError.audioStreamTimedOut)
        }
    }

    @discardableResult
    private func scheduleNextBuffer(allowSilence: Bool = true) -> Bool {
        guard startedPlayout || scheduledBufferCount < jitterBuffer.targetPacketCount else { return false }
        if let packet = jitterBuffer.next() {
            schedule(packet)
            packetPool.append(packet)
            packetsReceived += 1
        } else {
            guard allowSilence else { return false }
            let now = DispatchTime.now().uptimeNanoseconds
            jitterController?.onUnderrun(nowNanos: now)
            if let target = jitterController?.targetPacketCount {
                jitterBuffer.updateTarget(target)
            }
            schedule(nil)
            packetsLost += 1
        }
        if (packetsReceived + packetsLost).isMultiple(of: 100) {
            publishStats()
        }
        return true
    }

    private func schedule(_ packet: RTPPacket?) {
        guard let audio, let buffer = audioBufferPool.popLast() else {
            onFailure?(LanPulseError.audioPlaybackFailed)
            return
        }
        guard let channels = buffer.floatChannelData else {
            audioBufferPool.append(buffer)
            onFailure?(LanPulseError.audioPlaybackFailed)
            return
        }
        let frameCount = Int(buffer.frameLength)
        if let packet {
            packet.withUnsafePayloadBytes { payload in
                guard let source = payload.baseAddress?.assumingMemoryBound(to: Int16.self) else { return }
                var scale = Float(1.0 / 32_768.0)
                for channel in 0..<audio.channels {
                    let destination = channels[channel]
                    vDSP_vflt16(
                        source.advanced(by: channel),
                        vDSP_Stride(audio.channels),
                        destination,
                        1,
                        vDSP_Length(frameCount)
                    )
                    vDSP_vsmul(
                        destination,
                        1,
                        &scale,
                        destination,
                        1,
                        vDSP_Length(frameCount)
                    )
                }
            }
        } else {
            for channel in 0..<audio.channels {
                vDSP_vclr(channels[channel], 1, vDSP_Length(frameCount))
            }
        }
        scheduledBufferCount += 1
        let generation = playbackGeneration
        player.scheduleBuffer(buffer, completionCallbackType: .dataConsumed) { [weak self, buffer] _ in
            guard let self else { return }
            self.queue.async {
                guard generation == self.playbackGeneration else { return }
                self.audioBufferPool.append(buffer)
                self.scheduledBufferCount = max(0, self.scheduledBufferCount - 1)
                guard !self.stopped, self.startedPlayout else { return }
                if self.scheduledBufferCount < self.jitterBuffer.targetPacketCount {
                    self.scheduleNextBuffer()
                }
            }
        }
    }

    private func recycleDiscardedPackets() {
        while let packet = jitterBuffer.takeRecycledPacket() {
            packetPool.append(packet)
        }
    }

    private func startPlayoutIfReady() {
        guard !stopped, !interrupted, !startedPlayout, jitterBuffer.isReady else { return }
        jitterBuffer.startNearLive()
        recycleDiscardedPackets()
        startedPlayout = true
        for _ in 0..<jitterBuffer.targetPacketCount {
            scheduleNextBuffer()
        }
        player.play()
        publishStats()
    }

    private func growScheduledQueueFromBufferedPackets() {
        guard startedPlayout, !interrupted else { return }
        while scheduledBufferCount < jitterBuffer.targetPacketCount {
            guard scheduleNextBuffer(allowSilence: false) else { return }
        }
    }

    private func configureAudioSession(audio: AudioConfig) throws {
        let session = AVAudioSession.sharedInstance()
        try session.setCategory(.playback, mode: .default, options: [])
        try session.setPreferredSampleRate(Double(audio.sampleRate))
        try session.setPreferredIOBufferDuration(Double(audio.packetMs) / 1_000)
        try session.setActive(true)
    }

    private func rebuildAudioGraph() throws {
        guard let audio, let format else { throw LanPulseError.unsupportedAudio }
        playbackGeneration &+= 1
        player.stop()
        engine.stop()

        let nextEngine = AVAudioEngine()
        let nextPlayer = AVAudioPlayerNode()
        nextEngine.attach(nextPlayer)
        nextEngine.connect(nextPlayer, to: nextEngine.mainMixerNode, format: format)
        nextEngine.prepare()
        try nextEngine.start()

        engine = nextEngine
        player = nextPlayer
        audioBufferPool = try makeAudioBufferPool(format: format, audio: audio)
        scheduledBufferCount = 0
        startedPlayout = false
        startPlayoutIfReady()
    }

    private func observeAudioSession() {
        let center = NotificationCenter.default
        let session = AVAudioSession.sharedInstance()
        notificationTokens = [
            center.addObserver(
                forName: AVAudioSession.interruptionNotification,
                object: session,
                queue: nil
            ) { [weak self] notification in
                self?.queue.async { self?.handleInterruption(notification) }
            },
            center.addObserver(
                forName: AVAudioSession.routeChangeNotification,
                object: session,
                queue: nil
            ) { [weak self] notification in
                self?.queue.async { self?.handleRouteChange(notification) }
            },
            center.addObserver(
                forName: AVAudioSession.mediaServicesWereLostNotification,
                object: session,
                queue: nil
            ) { [weak self] _ in
                self?.queue.async { self?.pauseForAudioReset() }
            },
            center.addObserver(
                forName: AVAudioSession.mediaServicesWereResetNotification,
                object: session,
                queue: nil
            ) { [weak self] _ in
                self?.queue.async { self?.recoverAudioSession() }
            },
        ]
    }

    private func handleInterruption(_ notification: Notification) {
        guard !stopped,
              let rawType = notification.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
              let type = AVAudioSession.InterruptionType(rawValue: rawType)
        else { return }
        if type == .began {
            interrupted = true
            player.pause()
            engine.pause()
            return
        }

        recoverAudioSession()
    }

    private func handleRouteChange(_ notification: Notification) {
        guard !stopped, !interrupted,
              let rawReason = notification.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt,
              let reason = AVAudioSession.RouteChangeReason(rawValue: rawReason),
              reason == .oldDeviceUnavailable || reason == .newDeviceAvailable
        else { return }
        recoverAudioSession()
    }

    private func pauseForAudioReset() {
        guard !stopped else { return }
        interrupted = true
        player.stop()
        engine.stop()
    }

    private func recoverAudioSession() {
        guard !stopped, let audio else { return }
        do {
            try configureAudioSession(audio: audio)
            interrupted = false
            lastPacketAt = DispatchTime.now().uptimeNanoseconds
            try rebuildAudioGraph()
        } catch {
            onFailure?(error)
        }
    }

    private func makeAudioBufferPool(format: AVAudioFormat, audio: AudioConfig) throws -> [AVAudioPCMBuffer] {
        let count = jitterBuffer.maximumPacketCount + 4
        var buffers: [AVAudioPCMBuffer] = []
        buffers.reserveCapacity(count)
        for _ in 0..<count {
            guard let buffer = AVAudioPCMBuffer(
                pcmFormat: format,
                frameCapacity: AVAudioFrameCount(audio.framesPerPacket)
            ) else { throw LanPulseError.unsupportedAudio }
            buffer.frameLength = buffer.frameCapacity
            buffers.append(buffer)
        }
        return buffers
    }

    private func publishStats() {
        guard let audio else { return }
        onStats?(
            ReceiverStats(
                packetsReceived: packetsReceived,
                packetsLost: packetsLost,
                bufferMs: (jitterController?.targetPacketCount ?? jitterBuffer.targetPacketCount)
                    * audio.packetMs
            )
        )
    }
}
