// Avisos de licencias de terceros de lo que se distribuye (HU-022): crates Rust de los binarios
// (cargo-about, configurado en about.toml/about.hbs) y paquetes npm de producción que van dentro
// de la interfaz. Escribe THIRD-PARTY-LICENSES.md; con --check falla si el archivo versionado no
// coincide (una dependencia cambió sin regenerarlo).
//
// Requiere cargo-about 0.9.3 (`cargo install cargo-about --locked --version 0.9.3 --features cli`)
// en PATH, o su ruta en CARGO_ABOUT.
import { execFileSync } from "node:child_process";
import { readFileSync, readdirSync, writeFileSync, existsSync } from "node:fs";
import { join, relative } from "node:path";

const root = join(import.meta.dirname, "..");
const output = join(root, "THIRD-PARTY-LICENSES.md");
const check = process.argv.includes("--check");

// Mismo criterio que about.toml: otra licencia exige una decisión del propietario.
const ACCEPTED_NPM = new Set([
  "MIT",
  "ISC",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "0BSD",
]);

function rustSection() {
  const cargoAbout = process.env.CARGO_ABOUT ?? "cargo-about";
  return execFileSync(
    cargoAbout,
    ["generate", "--workspace", "--locked", "--offline", "--fail", "about.hbs"],
    {
      cwd: root,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "inherit"],
      maxBuffer: 64 << 20,
    },
  );
}

function npmPackages() {
  const paths = execFileSync(
    "npm",
    ["ls", "--omit=dev", "--all", "--parseable", "-w", "apps/desktop"],
    { cwd: root, encoding: "utf8", shell: process.platform === "win32" },
  )
    .split(/\r?\n/)
    .filter(Boolean);
  const seen = new Map();
  for (const dir of paths) {
    const manifest = join(dir, "package.json");
    if (!existsSync(manifest)) continue;
    const pkg = JSON.parse(readFileSync(manifest, "utf8"));
    // La raíz y el propio workspace no son de terceros.
    if (dir === root || pkg.name === "@kanbania/desktop") continue;
    const id = `${pkg.name}@${pkg.version}`;
    if (seen.has(id)) continue;
    const license = typeof pkg.license === "string" ? pkg.license : "";
    // Expresiones SPDX simples «A OR B»: basta una alternativa aceptada (no se admite AND).
    const options = license
      .replace(/[()]/g, "")
      .split(" OR ")
      .map((l) => l.trim());
    if (
      license.includes(" AND ") ||
      !options.some((l) => ACCEPTED_NPM.has(l))
    ) {
      throw new Error(
        `Licencia npm no aceptada: ${id} (${license || "sin campo license"})`,
      );
    }
    const file = readdirSync(dir).find((f) =>
      /^(licen[cs]e|copying)(\.|$)/i.test(f),
    );
    if (!file) {
      throw new Error(
        `${id} no incluye archivo de licencia (${relative(root, dir)})`,
      );
    }
    const text = readFileSync(join(dir, file), "utf8")
      .replace(/\r\n/g, "\n")
      .trim();
    seen.set(id, { id, license, text });
  }
  return [...seen.values()].sort((a, b) => a.id.localeCompare(b.id));
}

function npmSection(packages) {
  // Agrupa por texto idéntico, como cargo-about.
  const groups = new Map();
  for (const p of packages) {
    const group = groups.get(p.text) ?? { license: p.license, ids: [] };
    group.ids.push(p.id);
    groups.set(p.text, group);
  }
  let out = "";
  for (const [text, group] of groups) {
    out += `### ${group.license}\n\nUsada por:\n${group.ids.map((id) => `- ${id}`).join("\n")}\n\n`;
    out += "```text\n" + text.replaceAll("```", "'''") + "\n```\n\n";
  }
  return out;
}

const packages = npmPackages();
let content = `# Avisos de licencias de terceros

Kanban IA se distribuye con la licencia MIT OR Apache-2.0 (ver LICENSE-MIT y LICENSE-APACHE).
Incluye el software de terceros que se lista abajo, con sus avisos de copyright y licencias.
Archivo generado por \`npm run licenses\` (scripts/third-party-licenses.mjs): no lo edites a mano.

## SQLite

La app incluye SQLite (compilada desde el código fuente que trae \`libsqlite3-sys\`). SQLite es de
dominio público: https://sqlite.org/copyright.html

## Crates Rust (app de escritorio, servicio local y kanbania-mcp; macOS arm64 y Windows x64)

${rustSection().trim()}

## Paquetes npm incluidos en la interfaz (${packages.length})

${npmSection(packages).trim()}
`;

// Algunos textos de licencia traen CRLF: el archivo siempre va con LF.
content = content.replace(/\r\n?/g, "\n");

if (check) {
  const current = existsSync(output)
    ? readFileSync(output, "utf8").replace(/\r\n/g, "\n")
    : "";
  if (current !== content) {
    console.error(
      "THIRD-PARTY-LICENSES.md no está al día: ejecuta `npm run licenses`.",
    );
    process.exit(1);
  }
  console.log("THIRD-PARTY-LICENSES.md al día.");
} else {
  writeFileSync(output, content);
  console.log(
    `THIRD-PARTY-LICENSES.md: ${packages.length} paquetes npm + crates Rust.`,
  );
}
