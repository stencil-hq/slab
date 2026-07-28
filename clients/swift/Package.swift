// swift-tools-version: 6.0

import PackageDescription

let package = Package(
    name: "SlabSwift",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "Slab", targets: ["Slab"]),
        .library(name: "SlabAppKit", targets: ["SlabAppKit"]),
        .executable(name: "slab-swift", targets: ["SlabViewer"]),
    ],
    dependencies: [
        .package(url: "https://github.com/swiftwasm/WasmKit.git", exact: "0.2.1"),
        // WasmKit 0.2.1's open range selects swift-system 1.7, whose renamed
        // stat wrapper does not compile against that release.
        .package(url: "https://github.com/apple/swift-system", exact: "1.6.6"),
    ],
    targets: [
        .target(
            name: "Slab",
            dependencies: [
                .product(name: "WasmKit", package: "WasmKit"),
                .product(name: "SystemPackage", package: "swift-system"),
            ],
            resources: [.copy("Resources/slab_abi.wasm")]
        ),
        .target(name: "SlabAppKit", dependencies: ["Slab"]),
        .executableTarget(name: "SlabViewer", dependencies: ["Slab", "SlabAppKit"]),
    ]
)
