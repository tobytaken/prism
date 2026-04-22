import path from "node:path";

import { MakerDeb } from "@electron-forge/maker-deb";
import { MakerFlatpak } from "@electron-forge/maker-flatpak";
import { MakerRpm } from "@electron-forge/maker-rpm";
import { ElectronegativityPlugin } from "@electron-forge/plugin-electronegativity";
import { FusesPlugin } from "@electron-forge/plugin-fuses";
import { VitePlugin } from "@electron-forge/plugin-vite";
import type { ForgeConfig } from "@electron-forge/shared-types";
import { FuseV1Options, FuseVersion } from "@electron/fuses";

const appId = "com.toby.prism_electron_dev";
const forgeEntrypoint = path.basename(process.argv[1] ?? "");
const isForgeStart = forgeEntrypoint.startsWith("electron-forge-start");

const packagingPlugins = [
  new ElectronegativityPlugin({}),
  new FusesPlugin({
    version: FuseVersion.V1,
    [FuseV1Options.RunAsNode]: false,
    [FuseV1Options.EnableCookieEncryption]: true,
    [FuseV1Options.EnableNodeOptionsEnvironmentVariable]: false,
    [FuseV1Options.EnableNodeCliInspectArguments]: false,
    [FuseV1Options.EnableEmbeddedAsarIntegrityValidation]: true,
    [FuseV1Options.OnlyLoadAppFromAsar]: true,
  }),
];

const config: ForgeConfig = {
  packagerConfig: {
    asar: true,
    executableName: "prism-electron",
    name: "prism-electron",
    appBundleId: appId,
    icon: path.resolve(__dirname, "assets/icon.png"),
    extraResource: [
      path.resolve(__dirname, "rust/target/release/prism_backend"),
    ],
  },
  rebuildConfig: {},
  makers: [
    new MakerDeb({
      options: {
        name: "prism-electron",
        productName: "Prism Electron",
        maintainer: "Tobias Steinhaeusler",
        homepage: "https://example.com/prism-electron",
      },
    }),
    new MakerRpm({
      options: {
        name: "prism-electron",
        productName: "Prism Electron",
        homepage: "https://example.com/prism-electron",
      },
    }),
    new MakerFlatpak({
      options: {
        id: appId,
        productName: "Prism Electron",
        genericName: "GPU Manager",
        description: "Linux GPU management UI for NVIDIA hardware",
        icon: path.resolve(__dirname, "assets/icon.png"),
        categories: ["Utility", "System"],
        finishArgs: [
          "--share=ipc",
          "--socket=wayland",
          "--socket=fallback-x11",
          "--device=dri",
          "--filesystem=/run/prism",
        ],
        files: [],
      },
    }),
  ],
  plugins: [
    new VitePlugin({
      build: [
        {
          entry: "electron/main.ts",
          config: "vite.main.config.ts",
          target: "main",
        },
        {
          entry: "electron/preload.ts",
          config: "vite.preload.config.ts",
          target: "preload",
        },
      ],
      renderer: [
        {
          name: "main_window",
          config: "vite.renderer.config.ts",
        },
      ],
    }),
    // These only matter for packaged artifacts and can confuse Forge start
    // when the package manager installs multiple plugin-base copies.
    ...(isForgeStart ? [] : packagingPlugins),
  ],
};

export default config;
