// El binario `kanbania-mcp` no debe enlazar el núcleo ni SQLite: así no puede convertirse en
// otro propietario de los datos (HU-013). Compilado con `-p kanbania-mcp`, el árbol de
// dependencias normales no debe contenerlos.
import { execFileSync } from "node:child_process";

const tree = execFileSync(
  "cargo",
  [
    "tree",
    "-p",
    "kanbania-mcp",
    "-e",
    "normal",
    "--locked",
    "--prefix",
    "none",
  ],
  { encoding: "utf8" },
);
const forbidden = ["kanbania-core", "sqlx", "libsqlite3-sys", "axum"];
const found = forbidden.filter((name) =>
  tree.split("\n").some((line) => line.startsWith(`${name} `)),
);
if (found.length > 0) {
  console.error(
    `kanbania-mcp enlaza dependencias prohibidas: ${found.join(", ")}`,
  );
  process.exit(1);
}
console.log(
  "kanbania-mcp: sin núcleo, SQLite ni servidor HTTP en sus dependencias.",
);
