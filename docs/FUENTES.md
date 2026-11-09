# Fuentes y decisiones

Consultadas el 2026-11-09. Las recomendaciones de arquitectura y las historias son una propuesta propia para este producto; no requisitos impuestos por estas fuentes.

- [Claude Code: subagentes](https://code.claude.com/docs/en/sub-agents): ubicación de perfiles, frontmatter y restricciones de herramientas. Se usan name, description y tools; se omite model para heredar la elección del usuario.
- [Claude Code: buenas prácticas](https://code.claude.com/docs/en/best-practices): contexto enfocado y comprobaciones verificables. Las instrucciones Markdown no sustituyen controles de permisos ni pruebas.
- [Tauri: modelo de procesos](https://v2.tauri.app/concept/process-model/): separación del host Rust y la interfaz web.
- [Tauri: distribución](https://v2.tauri.app/distribute/): requisitos específicos de cada plataforma; verificar firma y empaquetado al llegar a M4.
- [SQLite: WAL](https://sqlite.org/wal.html): concurrencia local y límites del almacenamiento en red.
- [Vibe Kanban: separación de logs](https://vibekanban.com/blog/goodbye-sqlite-for-logs): experiencia documentada, no benchmark universal.
- [Agent Client Protocol](https://agentclientprotocol.com/get-started/introduction): comunicación cliente-agente; verificar capacidades por implementación.
- [Model Context Protocol](https://modelcontextprotocol.io/docs/getting-started/intro): consultar documentación y SDK oficiales vigentes al implementar MCP; fijar versión compatible y comprobar cliente real.

No se fijan versiones de proveedores o SDK por memoria. HU-001 define el stack reproducible y HU-013/HU-019 registran compatibilidad de protocolos. No se presupone que un producto de chat tenga API, que una suscripción cubra consumo de API o que una marca implemente ACP.
