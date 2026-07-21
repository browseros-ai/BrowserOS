import Foundation
import SwiftUI

/// Data models for the trios-mesh UI. Mirrors the JSON emitted by clade-meshd.

struct MeshHealth: Codable {
    let status: String
    let node_id: UInt32
}

struct MeshStatus: Codable {
    let node_id: UInt32
    let neighbors: [MeshNeighbor]
    let routes: [MeshRoute]
    let sessions: [MeshSession]
    let metrics: MeshMetrics
}

struct MeshNeighbor: Codable, Identifiable {
    let id: UInt32
    let etx: Float
    let etx_label: String

    var statusColor: MeshStatusColor {
        switch etx_label {
        case "perfect", "good": return .green
        case "fair": return .yellow
        case "poor": return .orange
        default: return .red
        }
    }
}

struct MeshRoute: Codable, Identifiable {
    let destination: UInt32
    let next_hop: UInt32?
    let path_etx: Float?

    var id: UInt32 { destination }
}

struct MeshSession: Codable, Identifiable {
    let peer: UInt32
    let has_session: Bool

    var id: UInt32 { peer }
}

struct MeshMetrics: Codable {
    let link_loss_to_reroute_ms: Float?
    let node_off_to_reroute_ms: Float?
}

enum MeshStatusColor {
    case green, yellow, orange, red

    var swiftUIColor: Color {
        switch self {
        case .green: return Color.green
        case .yellow: return Color.yellow
        case .orange: return Color.orange
        case .red: return Color.red
        }
    }
}

struct MeshObserveRequest: Codable {
    let peer: UInt32
    let we_heard: Bool
    let they_heard: Bool
}

struct MeshHelloRequest: Codable {
    let peer: UInt32
    let seq: UInt32
    let heard: [UInt32]
}

struct MeshPeerRequest: Codable {
    let peer: UInt32
}

struct MeshSimpleResponse: Codable {
    let ok: Bool
}
