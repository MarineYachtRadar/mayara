# Mayara SignalK WASM Plugin

SignalK WASM plugin for radar detection and display.

## WebApp Renderers

The webapp supports multiple rendering modes, accessible via URL parameter:

| URL Parameter | Renderer | Description |
|---------------|----------|-------------|
| (default) | WebGL | GPU-accelerated, texture-based |
| `?draw=alt` | WebGL Alt | GPU-accelerated, line-based |
| `?draw=2d` | Canvas 2D | CPU-based fallback |
| `?draw=webgpu` | WebGPU | Next-gen GPU API |

## WebGPU Requirements

WebGPU requires specific browser configuration to work:

### 1. Hardware Acceleration

WebGPU requires hardware acceleration to access the GPU.

**Chrome/Edge:**
1. Go to `chrome://settings/system` (or `edge://settings/system`)
2. Enable **"Use hardware acceleration when available"**
3. Restart the browser

### 2. Secure Context (HTTPS or localhost)

WebGPU is only available in secure contexts for security reasons.

- **localhost** - Works with HTTP (e.g., `http://localhost:3000`)
- **Remote IP** - Requires HTTPS (e.g., `https://192.168.0.10:3000`)

If accessing SignalK on a remote machine via IP address, you have several options:

**Option 1: SSH Port Forwarding** (recommended, no SSL setup needed)
```bash
ssh -L 3000:localhost:3000 user@192.168.0.10
```
Then access via `http://localhost:3000`

**Option 2: Chrome Flags for Insecure Origins**

Enable these flags in `chrome://flags`:

1. **Unsafe WebGPU Support** (`chrome://flags/#enable-unsafe-webgpu`)
   - Set to **Enabled**

2. **Insecure origins treated as secure** (`chrome://flags/#unsafely-treat-insecure-origin-as-secure`)
   - Add your SignalK server URL: `http://192.168.0.10:3000`

3. Restart Chrome

**Option 3: HTTPS/SSL**
- Configure SignalK with HTTPS/SSL certificates

### 3. Browser Support

| Browser | Version | Notes |
|---------|---------|-------|
| Chrome | 113+ | Stable support |
| Edge | 113+ | Stable support |
| Firefox | 120+ | Enable `dom.webgpu.enabled` in `about:config` |
| Safari | 17+ | macOS Sonoma |

### Troubleshooting

Check WebGPU availability in browser console:
```javascript
navigator.gpu  // Should return GPU object, not undefined
```

If `undefined`, WebGPU is not available due to one of the above requirements not being met.
