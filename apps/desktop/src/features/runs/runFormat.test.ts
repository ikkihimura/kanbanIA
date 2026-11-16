// HU-021: cada agente ACP y cada ejecutor tienen nombre, etiqueta y agente asociados. Si el
// núcleo añade un agente o un ejecutor, esta prueba deja de compilar o de pasar.
import { describe, expect, it } from "vitest";
import type { AcpAgent } from "../../contracts/generated/AcpAgent";
import type { RunExecutor } from "../../contracts/generated/RunExecutor";
import {
  ACP_AGENT_NAME,
  acpAgentOf,
  executorLabel,
  TEST_PROCESS_LABEL,
} from "./runFormat";

/** Ejecutor de ejemplo por cada `kind`; `satisfies` exige que estén todos. */
const EXAMPLES = {
  testProcess: { kind: "testProcess", scenario: "success" },
  claudeAcp: { kind: "claudeAcp" },
  codexAcp: { kind: "codexAcp" },
} as const satisfies {
  [K in RunExecutor["kind"]]: Extract<RunExecutor, { kind: K }>;
};

/** Agente esperado de cada ejecutor; `satisfies` exige todos los `kind`. */
const EXPECTED_AGENT = {
  testProcess: null,
  claudeAcp: "claudeCode",
  codexAcp: "codex",
} as const satisfies Record<RunExecutor["kind"], AcpAgent | null>;

// Todo `AcpAgent` tiene un ejecutor (comprobación de tipos: `never` si no falta ninguno).
type Uncovered = Exclude<
  AcpAgent,
  (typeof EXPECTED_AGENT)[keyof typeof EXPECTED_AGENT]
>;
const everyAgentHasExecutor: [Uncovered] extends [never] ? true : false = true;

describe("agentes y ejecutores ACP", () => {
  it("cada agente tiene nombre y un ejecutor que lo lanza", () => {
    expect(everyAgentHasExecutor).toBe(true);
    const agents = Object.keys(ACP_AGENT_NAME) as AcpAgent[];
    expect(agents.sort()).toEqual(["claudeCode", "codex"]);
    for (const agent of agents) {
      expect(ACP_AGENT_NAME[agent]).not.toBe("");
      expect(Object.values(EXPECTED_AGENT)).toContain(agent);
    }
  });

  it("acpAgentOf y executorLabel cubren todos los ejecutores", () => {
    for (const [kind, executor] of Object.entries(EXAMPLES)) {
      const expected = EXPECTED_AGENT[kind as RunExecutor["kind"]];
      expect(acpAgentOf(executor)).toBe(expected);
      const label = executorLabel(executor);
      if (expected === null) expect(label).toBe(TEST_PROCESS_LABEL);
      else expect(label).toBe(`${ACP_AGENT_NAME[expected]} (ACP)`);
    }
  });
});
