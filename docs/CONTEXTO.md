# Contexto de producto

Fecha: 2026-11-09. Nombre provisional: Kanban IA. Este documento es una propuesta ejecutable basada en la planificación con el propietario; las decisiones de licencia, marca y distribución comercial permanecen abiertas.

## Problema y propuesta
Un desarrollador necesita conectar planificación, código y actividad de múltiples agentes sin perder control del trabajo. La aplicación permite ver qué historia ejecuta cada agente, qué necesita intervención y qué resultado entregó. La estética retro aporta identidad, pero la precisión del estado y la revisión humana tienen prioridad.

## Público y plataformas
Primero desarrolladores individuales y pequeños equipos que trabajan localmente. macOS y Windows; Linux fuera del compromiso inicial. Sin cuenta obligatoria. Planificación disponible offline; inferencia remota depende de la conexión y del proveedor. No prometer que una suscripción de chat habilita APIs o agentes.

## Experiencia
- Board con columnas configurables, límites WIP y swimlanes por épica o agente.
- Épicas con historias, progreso basado en historias aceptadas y dependencias explícitas.
- Equipo con avatar del usuario y agentes con nombre estable, rol, proveedor y modelo separados.
- Detalle de historia con descripción Markdown, criterios verificables, prioridad, responsable humano, agente asignado, dependencias y resultados.
- Bandeja de atención con bloqueos, entregas y solicitudes de aprobación.
- Avatar por piezas y sprites originales; no depender de generación de imágenes ni de logos oficiales para el MVP. Importar imágenes personalizadas puede esperar.
- Tipografía legible, estados con etiquetas además de color, navegación por teclado, sonidos desactivados inicialmente y movimiento reducido.

## Vocabulario e invariantes
- Proyecto: planificación que puede vincular varios repositorios.
- Columna: etapa de flujo. Swimlane: agrupación visual, no un estado adicional.
- Historia: unidad de valor; puede tener subtareas y múltiples intentos de ejecución.
- Agente: identidad configurada con rol y capacidades; no equivale a modelo.
- Ejecución: intento sobre una instantánea inmutable de la historia, configuración y contexto.
- Ejecución completada no equivale a historia aceptada.
- Actividad MCP es última interacción observada, no prueba de proceso vivo.
- Tokens o costos desconocidos se muestran como no disponibles, nunca cero ficticio.

## Alcance por hitos
M0: base ejecutable, arquitectura y almacenamiento.
M1: planificación local usable, avatares, portabilidad y atención.
M2: MCP para agentes externos; primer MVP funcional integrado con IA.
M3: supervisión local + primer adaptador ACP compatible + segundo adaptador que demuestre extensibilidad.
M4: alpha distribuible y documentación comunitaria.

## Fuera de alcance inicial
SaaS, colaboración simultánea entre máquinas, base SQLite en carpeta de red, facturación, selección automática del modelo más barato, marketplace, ejecución autónoma ilimitada, sincronización automática con GitHub y editor de código completo.

## Validación del producto
Medir en sesiones de prueba: tiempo hasta crear primera historia, capacidad de identificar un bloqueo, completar el flujo crear→asignar→entregar→aceptar y recuperar el trabajo tras reiniciar. No añadir telemetría remota por defecto. Cualquier objetivo numérico se define después de una línea base.

## Decisiones pendientes que no bloquean M0–M2
Marca definitiva, licencia, firma de instaladores, proveedor/modelo específico para M3 y política comercial. Usar artefactos originales y documentar licencias de dependencias. No publicar hasta resolver distribución y licencia.
