# Bitácora — formato de las entradas

Página pública: `https://<owner>.github.io/<repo>/bitacora.html` (la URL concreta está en la sección **URLs vivas** de `CLAUDE.md`).

Cada sesión de trabajo **añade un fichero nuevo** en esta carpeta, con nombre `AAAAMMDD-HHMM.json` (hora de Madrid). Nunca se editan ni se borran entradas anteriores, y nunca se tocan `docs/bitacora.html` ni `docs/bitacora/index.json`.

`index.json` lo genera `.github/workflows/pages.yml` en cada publicación (lee todos los `*.json` de aquí, los valida y los ordena de más nuevo a más antiguo). No se commitea: está en `.gitignore`. Si una entrada no es JSON válido o le falta `fecha` o `titulo`, el run de Pages falla.

## Campos

| Campo | Tipo | Obligatorio | Qué es |
|---|---|---|---|
| `fecha` | texto ISO con zona | sí | `2026-09-08T18:30:00+02:00` |
| `titulo` | texto | sí | Una línea |
| `objetivo` | texto | | Para qué era la sesión |
| `prompts` | array de `{texto, nota?}` | | Transcripción fiel de lo que pidió el usuario, en sus términos |
| `cambios` | array de texto | | Qué cambió, fichero por fichero |
| `sha` / `sha_completo` | texto | | Commit principal de la sesión (7 y 40 caracteres) |
| `run` | texto | | Id del run de Actions que lo publicó |
| `mock` | ruta relativa a `docs/` | | Mock archivado de esa sesión (`mocks/NNN-nombre.html`) |
| `build` | texto | | Número de build del mock |
| `pagina` | ruta relativa a `docs/` | | Página a enlazar |
| `fly` | URL o `null` | | `/salud` si el backend estaba activo |
| `pendiente` | array de texto | | Lo que queda |
| `enlaces` | array de `{texto, url}` | | Enlaces extra |
| `notas` | texto | | Avisos: transcripción aproximada, prompts resumidos, etc. |

**`docs/` es público.** Nunca copies prompts con claves, rutas internas, datos personales o nombres de clientes: resúmelos y anótalo en `notas`.

## Ejemplo

En la plantilla está además como `ejemplo.json`, para que su bitácora muestre una entrada. `init-plantilla.yml` lo borra al inicializar un proyecto hijo, que empieza con la bitácora vacía; este ejemplo queda aquí como referencia del esquema.

```json
{
  "fecha": "2026-09-08T18:30:00+02:00",
  "titulo": "Pantalla de inicio de sesión en el mock",
  "objetivo": "Añadir la primera pantalla real al mock y dejarla publicada para probarla desde el móvil.",
  "prompts": [
    {
      "texto": "Añade a docs/index.html una pantalla de inicio de sesión (usuario, contraseña, botón). Sube el build a 002, archiva el mock anterior en docs/mocks/ y publica. Dime cuándo puedo verlo.",
      "nota": "Transcripción de lo pedido en la sesión, no una copia literal palabra por palabra."
    },
    {
      "texto": "El botón queda muy pegado al campo de contraseña. Sepáralo y haz que ocupe todo el ancho en móvil."
    }
  ],
  "cambios": [
    "docs/index.html: formulario de acceso con usuario, contraseña y botón a ancho completo por debajo de 520 px.",
    "docs/mocks/001-inicio.html: archivado el mock anterior.",
    "Build actualizado a MP-B1-20260908-002."
  ],
  "sha": "1810265",
  "sha_completo": "1810265f3c2a9d84b71e05c6a2f8d3419b7e0c22",
  "run": "17482930561",
  "mock": "mocks/002-acceso.html",
  "build": "MP-B1-20260908-002",
  "pagina": "index.html",
  "fly": null,
  "pendiente": [
    "Decidir si el acceso lleva recuperación de contraseña",
    "Activar Fly cuando haya que validar credenciales de verdad"
  ],
  "enlaces": [],
  "notas": ""
}
```
