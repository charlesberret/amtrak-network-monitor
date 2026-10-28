import AppKit
import CoreLocation
import Foundation

/// Writes one location fix at a time for amtrak-network-monitor.
/// A terminal program cannot ask for Location. This app can.
final class Recorder: NSObject, CLLocationManagerDelegate {
    let manager = CLLocationManager()
    let geocoder = CLGeocoder()
    let file: URL
    var lastWritten = Date.distantPast

    init(file: URL) {
        self.file = file
        super.init()
    }

    func start() {
        let parent = file.deletingLastPathComponent()
        try? FileManager.default.createDirectory(at: parent, withIntermediateDirectories: true)
        if !FileManager.default.fileExists(atPath: file.path) {
            try? "time\tlat\tlon\tacc_m\tspeed_mps\tplace\n".write(to: file, atomically: true, encoding: .utf8)
        }
        manager.delegate = self
        manager.desiredAccuracy = kCLLocationAccuracyNearestTenMeters
        manager.distanceFilter = 40
        manager.activityType = .otherNavigation
        manager.pausesLocationUpdatesAutomatically = false
        manager.requestWhenInUseAuthorization()
        manager.startUpdatingLocation()
    }

    func locationManagerDidChangeAuthorization(_ manager: CLLocationManager) {
        switch manager.authorizationStatus {
        case .authorizedAlways, .authorized:
            manager.startUpdatingLocation()
        default:
            break
        }
    }

    func locationManager(_ manager: CLLocationManager, didUpdateLocations locations: [CLLocation]) {
        guard let loc = locations.last else { return }
        let now = Date()
        if now.timeIntervalSince(lastWritten) < 15 { return }
        lastWritten = now
        if geocoder.isGeocoding { geocoder.cancelGeocode() }
        geocoder.reverseGeocodeLocation(loc) { [weak self] marks, _ in
            self?.write(loc, place: Recorder.placeLine(marks?.first))
        }
    }

    func locationManager(_ manager: CLLocationManager, didFailWithError error: Error) {
        fputs("location: \(error.localizedDescription)\n", stderr)
    }

    func write(_ loc: CLLocation, place: String) {
        let stamp = ISO8601DateFormatter()
        stamp.formatOptions = [.withInternetDateTime]
        stamp.timeZone = TimeZone(secondsFromGMT: 0)
        let when = stamp.string(from: loc.timestamp)
        let speed = loc.speed >= 0 ? String(format: "%.1f", loc.speed) : "-"
        let safePlace = place.replacingOccurrences(of: "\t", with: " ").replacingOccurrences(of: "\n", with: " ")
        let line = String(
            format: "%@\t%.6f\t%.6f\t%.0f\t%@\t%@\n",
            when,
            loc.coordinate.latitude,
            loc.coordinate.longitude,
            loc.horizontalAccuracy,
            speed,
            safePlace
        )
        guard let data = line.data(using: .utf8) else { return }
        if let handle = try? FileHandle(forWritingTo: file) {
            _ = try? handle.seekToEnd()
            try? handle.write(contentsOf: data)
            try? handle.close()
        }
    }

    static func placeLine(_ mark: CLPlacemark?) -> String {
        guard let mark else { return "" }
        var parts: [String] = []
        if let name = mark.name, mark.locality != name {
            parts.append(name)
        }
        if let locality = mark.locality {
            parts.append(locality)
        }
        if let state = mark.administrativeArea {
            parts.append(state)
        }
        return parts.joined(separator: ", ")
    }
}

final class Stay: NSObject, NSApplicationDelegate {
    let recorder: Recorder
    init(recorder: Recorder) {
        self.recorder = recorder
    }
    func applicationDidFinishLaunching(_ notification: Notification) {
        recorder.start()
    }
}

let file = FileManager.default.homeDirectoryForCurrentUser
    .appendingPathComponent("Library/Application Support/amtrak-network-monitor", isDirectory: true)
    .appendingPathComponent("location.tsv")
let stay = Stay(recorder: Recorder(file: file))
let app = NSApplication.shared
app.setActivationPolicy(.regular)
app.delegate = stay
app.run()
