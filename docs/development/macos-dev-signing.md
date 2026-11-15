# macOS development signing and permissions

`pnpm build:dev` creates the development `.app` bundle with a stable local code-signing identity. This is necessary because macOS TCC associates Accessibility and Microphone authorization with the app identity, not just its bundle ID or output path. Re-signing each build ad hoc can invalidate the previous grant.

## First development build

Run:

```sh
pnpm build:dev
```

On the first macOS build, the script:

1. Creates a self-signed Code Signing certificate named `SayTypeTypeType Dev` if it is not already in the login Keychain.
2. Stores the private key in the login Keychain and permits Apple's `codesign` utility to use it.
3. Asks for administrator approval once to trust that certificate **only for code signing** in the System Keychain.
4. Builds the development app using that identity and verifies the resulting signature.

The certificate and private key are generated on the developer's Mac. They are not stored in the repository, shared with other developers, or used for release builds. The self-signed identity is for local development only and must never be used to distribute the app.

Because the signing identity changes from the previous ad-hoc signature, grant Accessibility and Microphone permissions to the newly signed development app once after the first successful build. Later `pnpm build:dev` rebuilds reuse the same trusted certificate and should preserve those grants.

## Diagnostics

Inspect the identity and designated requirement of the built app:

```sh
codesign -dv --verbose=4 "src-tauri/target/release/bundle/macos/SayTypeTypeType Dev.app"
codesign -dr - "src-tauri/target/release/bundle/macos/SayTypeTypeType Dev.app"
```

The first command should show `Authority=SayTypeTypeType Dev`. The designated requirement should retain the development bundle identifier and certificate identity across rebuilds.

The development bundle identifier is `com.yolanda.voicepaste.dev`; the production bundle identifier and release signing configuration are unchanged.

## Trust scope

The setup adds a local root certificate to the System Keychain with the `codeSign` trust policy only. This makes the certificate trusted for verifying code signatures on this Mac; it does not make it an Apple Developer ID certificate, enable notarization, or make the development app suitable for distribution. Removing the local certificate from Keychain will invalidate the stable signing identity and require another one-time permission grant.
