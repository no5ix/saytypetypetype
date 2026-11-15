/**
 * Build the macOS development app with a stable local code-signing identity.
 *
 * TCC binds Accessibility and Microphone grants to the app's code-signing
 * requirement. Ad-hoc signatures change with every rebuild, so macOS treats
 * each build as a new app. This script creates/trusts one local code-signing
 * certificate once, then Tauri reuses it for every development bundle.
 *
 * The private key stays in the user's login Keychain. The certificate is
 * trusted only for code signing in the local System Keychain; it is never
 * committed or used by production/release builds.
 */

import { spawnSync } from "node:child_process";
import fs from "node:fs";
import { homedir, tmpdir } from "node:os";
import path from "node:path";

const ROOT = path.resolve(__dirname, "..");
const SIGNING_IDENTITY = "SayTypeTypeType Dev";
const LOGIN_KEYCHAIN = path.join(homedir(), "Library/Keychains/login.keychain-db");
const SYSTEM_KEYCHAIN = "/Library/Keychains/System.keychain";
const APP_PATH = path.join(
  ROOT,
  "src-tauri/target/release/bundle/macos/SayTypeTypeType Dev.app",
);

function run(
  command: string,
  args: string[],
  options: { inherit?: boolean; env?: NodeJS.ProcessEnv } = {},
): string {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    stdio: options.inherit ? "inherit" : "pipe",
    env: options.env ?? process.env,
    shell: process.platform === "win32",
  });

  if (result.error) {
    throw new Error(`Could not run ${command}: ${result.error.message}`);
  }
  if (result.status !== 0) {
    const detail = [result.stdout, result.stderr].filter(Boolean).join("\n").trim();
    throw new Error(
      `${command} exited with code ${result.status ?? "unknown"}${detail ? `:\n${detail}` : ""}`,
    );
  }
  return [result.stdout, result.stderr].filter(Boolean).join("\n");
}

function succeeds(command: string, args: string[]): boolean {
  const result = spawnSync(command, args, { stdio: "ignore" });
  return !result.error && result.status === 0;
}

function findIdentity(): boolean {
  if (!fs.existsSync(LOGIN_KEYCHAIN)) {
    throw new Error(`Login Keychain not found: ${LOGIN_KEYCHAIN}`);
  }
  const output = run("security", [
    "find-identity",
    "-p",
    "codesigning",
    LOGIN_KEYCHAIN,
  ]);
  return output.includes(`"${SIGNING_IDENTITY}"`);
}

function exportCertificate(outputPath: string): void {
  const pem = run("security", [
    "find-certificate",
    "-c",
    SIGNING_IDENTITY,
    "-p",
    LOGIN_KEYCHAIN,
  ]);
  if (!pem.includes("-----BEGIN CERTIFICATE-----")) {
    throw new Error(`Could not export the ${SIGNING_IDENTITY} certificate from Keychain.`);
  }
  fs.writeFileSync(outputPath, pem, { mode: 0o600 });
}

function createIdentity(tempDir: string): void {
  const keyPath = path.join(tempDir, "dev-signing.key");
  const certPath = path.join(tempDir, "dev-signing.crt");
  const p12Path = path.join(tempDir, "dev-signing.p12");
  const opensslConfig = path.join(tempDir, "openssl.cnf");

  fs.writeFileSync(
    opensslConfig,
    `[req]
prompt = no
distinguished_name = distinguished_name
x509_extensions = code_signing
[distinguished_name]
CN = ${SIGNING_IDENTITY}
[ code_signing ]
basicConstraints = critical,CA:TRUE
keyUsage = critical,digitalSignature,keyCertSign
extendedKeyUsage = critical,codeSigning
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid:always
`,
    { mode: 0o600 },
  );

  console.log(`Creating the one-time local "${SIGNING_IDENTITY}" code-signing identity...`);
  run("openssl", [
    "req",
    "-x509",
    "-newkey",
    "rsa:2048",
    "-sha256",
    "-days",
    "3650",
    "-nodes",
    "-keyout",
    keyPath,
    "-out",
    certPath,
    "-subj",
    `/CN=${SIGNING_IDENTITY}`,
    "-config",
    opensslConfig,
    "-extensions",
    "code_signing",
  ]);

  run("openssl", [
    "pkcs12",
    "-export",
    "-inkey",
    keyPath,
    "-in",
    certPath,
    "-out",
    p12Path,
    "-passout",
    "pass:",
  ]);

  // Import the private key + certificate into the login Keychain and allow
  // Apple's codesign utility to use the key without recurring access prompts.
  run(
    "security",
    [
      "import",
      p12Path,
      "-k",
      LOGIN_KEYCHAIN,
      "-P",
      "",
      "-T",
      "/usr/bin/codesign",
    ],
    { inherit: true },
  );
}

function ensureTrustedForCodeSigning(certPath: string): void {
  if (
    succeeds("security", [
      "verify-cert",
      "-c",
      certPath,
      "-p",
      "codeSign",
    ])
  ) {
    return;
  }

  console.log(
    [
      "",
      "One-time macOS setup: trust the local development certificate for code signing.",
      "macOS will request administrator approval. This does not install a production",
      "certificate; it trusts this machine's local SayTypeTypeType development identity.",
      "",
    ].join("\n"),
  );

  run(
    "sudo",
    [
      "security",
      "add-trusted-cert",
      "-d",
      "-r",
      "trustRoot",
      "-p",
      "codeSign",
      "-k",
      SYSTEM_KEYCHAIN,
      certPath,
    ],
    { inherit: true },
  );

  if (
    !succeeds("security", [
      "verify-cert",
      "-c",
      certPath,
      "-p",
      "codeSign",
    ])
  ) {
    throw new Error(
      "The local signing certificate is still not trusted for code signing. Check Keychain trust settings and try again.",
    );
  }
}

function ensureMacSigningIdentity(): void {
  const tempDir = fs.mkdtempSync(path.join(tmpdir(), "saytypetypetype-signing-"));
  try {
    if (!findIdentity()) {
      createIdentity(tempDir);
    }

    if (!findIdentity()) {
      throw new Error(
        `The ${SIGNING_IDENTITY} identity was not found in the login Keychain after import.`,
      );
    }

    const certPath = path.join(tempDir, "dev-signing.crt");
    exportCertificate(certPath);
    ensureTrustedForCodeSigning(certPath);
    console.log(`Using persistent local signing identity: ${SIGNING_IDENTITY}`);
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

function buildTauri(): void {
  const tauriBin = path.join(
    ROOT,
    "node_modules/.bin",
    process.platform === "win32" ? "tauri.cmd" : "tauri",
  );
  const args = [
    "build",
    "--config",
    "src-tauri/tauri.dev.conf.json",
    "--bundles",
    "app",
  ];

  console.log(`\n> ${tauriBin} ${args.join(" ")}\n`);
  run(tauriBin, args, {
    inherit: true,
    env: {
      ...process.env,
      ...(process.platform === "darwin"
        ? { APPLE_SIGNING_IDENTITY: SIGNING_IDENTITY }
        : {}),
    },
  });
}

function verifyMacBundle(): void {
  if (!fs.existsSync(APP_PATH)) {
    throw new Error(`Expected development app bundle was not created: ${APP_PATH}`);
  }

  run("codesign", ["--verify", "--deep", "--strict", "--verbose=2", APP_PATH], {
    inherit: true,
  });
  const details = run("codesign", ["-dv", "--verbose=4", APP_PATH]);
  if (!details.includes(`Authority=${SIGNING_IDENTITY}`)) {
    throw new Error(
      `The app bundle is not signed with the expected identity "${SIGNING_IDENTITY}".\n${details}`,
    );
  }
  console.log(`\nVerified stable signature: ${SIGNING_IDENTITY}`);
}

function main(): void {
  if (process.platform === "darwin") {
    ensureMacSigningIdentity();
  }

  buildTauri();

  if (process.platform === "darwin") {
    verifyMacBundle();
    console.log(
      [
        "",
        "Development build complete.",
        "If this is the first build using the new signing identity, grant Accessibility",
        "and Microphone permissions once more. Future rebuilds reuse the same trusted",
        "identity and should no longer invalidate those grants.",
      ].join("\n"),
    );
  }
}

try {
  main();
} catch (error) {
  console.error(`\nDevelopment build failed: ${(error as Error).message}`);
  process.exit(1);
}
