import ts from "typescript";
import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";

const root = process.cwd();
const out = join(root, "dist");
await mkdir(join(out, "assets"), { recursive: true });

async function compile(name: "main" | "preload" | "renderer"): Promise<void> {
  const source = await readFile(join(root, "src", `${name}.ts`), "utf8");
  const result = ts.transpileModule(source, {
    fileName: `${name}.ts`,
    compilerOptions: {
      target: ts.ScriptTarget.ES2022,
      module: name === "renderer" ? ts.ModuleKind.ES2022 : ts.ModuleKind.CommonJS
    }
  });
  await writeFile(join(out, `${name}.js`), result.outputText, "utf8");
}

await Promise.all([
  compile("main"), compile("preload"), compile("renderer"),
  copyFile(join(root, "src", "index.html"), join(out, "index.html")),
  copyFile(join(root, "src", "styles.css"), join(out, "styles.css")),
  copyFile(join(root, "assets", "logo.png"), join(out, "assets", "logo.png")),
  copyFile(join(root, "assets", "logo.ico"), join(out, "assets", "logo.ico"))
]);

