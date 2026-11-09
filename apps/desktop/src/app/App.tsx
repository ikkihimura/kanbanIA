import { AppInfoPanel } from "../features/app-info/AppInfoPanel";

export function App() {
  return (
    <main className="shell">
      <header className="shell__header">
        <h1 className="shell__title">
          Kanban IA
          <span className="shell__cursor" aria-hidden="true" />
        </h1>
        <p className="shell__subtitle">
          Base desktop lista. El tablero llegará en los próximos hitos.
        </p>
      </header>
      <AppInfoPanel />
    </main>
  );
}
