# Web Dashboard

A browser-based dashboard with live updates. The frontend is embedded in the binary: no Node.js or build step required.

```bash
scrutin -r web                   # binds to 127.0.0.1:7878
scrutin -r web:0.0.0.0:3000      # custom address
```

The dashboard uses server-sent events to stream results as they arrive. It binds to localhost only by default. If the port is busy, *Scrutin* tries the next one automatically.

![Web dashboard](../assets/screenshot_web_lint.png){ .screenshot }
