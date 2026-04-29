import { useEffect, useRef } from "react";
import * as THREE from "three";

const POSTER = { w: 27, h: 40, padding: 2, cols: 11, rows: 8 };

// Generate gradient+icon poster textures procedurally
function createPosterTexture(index: number): THREE.CanvasTexture {
  const canvas = document.createElement("canvas");
  canvas.width = 200;
  canvas.height = 300;
  const ctx = canvas.getContext("2d")!;

  // Gradient backgrounds — varied color combos
  const palettes = [
    ["#1a1a4e", "#4a1942"],
    ["#0f2027", "#2c5364"],
    ["#1d1145", "#6b21a8"],
    ["#0c0c2d", "#1e3a5f"],
    ["#2d1b4e", "#8b1a4a"],
    ["#0a192f", "#1a5276"],
    ["#1a0533", "#4c1d95"],
    ["#0d1117", "#3b0764"],
    ["#1e1b4b", "#581c87"],
    ["#0c4a6e", "#1e1b4b"],
    ["#3b0764", "#1e3a5f"],
    ["#1a1a2e", "#16213e"],
    ["#0f0c29", "#302b63"],
    ["#1b1b2f", "#462255"],
    ["#0a0a23", "#1a3c6e"],
    ["#120428", "#2d1854"],
    ["#1a0a2e", "#0a2540"],
    ["#0d0221", "#341948"],
    ["#1c0b2e", "#0b3d5f"],
    ["#2a0845", "#0d324d"],
  ];

  const pal = palettes[index % palettes.length];
  const grad = ctx.createLinearGradient(0, 0, canvas.width, canvas.height);
  grad.addColorStop(0, pal[0]);
  grad.addColorStop(1, pal[1]);
  ctx.fillStyle = grad;
  ctx.fillRect(0, 0, canvas.width, canvas.height);

  // Subtle grid lines
  ctx.strokeStyle = "rgba(255,255,255,0.03)";
  ctx.lineWidth = 1;
  for (let y = 0; y < canvas.height; y += 20) {
    ctx.beginPath();
    ctx.moveTo(0, y);
    ctx.lineTo(canvas.width, y);
    ctx.stroke();
  }

  // Play button / video icon in center
  const cx = canvas.width / 2;
  const cy = canvas.height / 2;

  // Circle
  ctx.beginPath();
  ctx.arc(cx, cy, 28, 0, Math.PI * 2);
  ctx.fillStyle = "rgba(255,255,255,0.06)";
  ctx.fill();
  ctx.strokeStyle = "rgba(255,255,255,0.12)";
  ctx.lineWidth = 2;
  ctx.stroke();

  // Triangle play
  ctx.beginPath();
  ctx.moveTo(cx - 8, cy - 14);
  ctx.lineTo(cx - 8, cy + 14);
  ctx.lineTo(cx + 16, cy);
  ctx.closePath();
  ctx.fillStyle = "rgba(255,255,255,0.1)";
  ctx.fill();

  // Fake title lines at bottom
  const lineY = canvas.height - 50;
  ctx.fillStyle = "rgba(255,255,255,0.08)";
  ctx.fillRect(20, lineY, canvas.width * 0.6, 8);
  ctx.fillRect(20, lineY + 16, canvas.width * 0.35, 6);

  const texture = new THREE.CanvasTexture(canvas);
  texture.colorSpace = THREE.SRGBColorSpace;
  return texture;
}

function roundedRect(
  shape: THREE.Shape,
  x: number,
  y: number,
  w: number,
  h: number,
  r: number,
) {
  shape.moveTo(x, y + r);
  shape.lineTo(x, y + h - r);
  shape.quadraticCurveTo(x, y + h, x + r, y + h);
  shape.lineTo(x + w - r, y + h);
  shape.quadraticCurveTo(x + w, y + h, x + w, y + h - r);
  shape.lineTo(x + w, y + r);
  shape.quadraticCurveTo(x + w, y, x + w - r, y);
  shape.lineTo(x + r, y);
  shape.quadraticCurveTo(x, y, x, y + r);
}

export default function PosterWall() {
  const containerRef = useRef<HTMLDivElement>(null);
  const cleanupRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    // Scene
    const scene = new THREE.Scene();
    const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.setClearColor(0x000000, 0);

    const canvasW = container.clientWidth;
    const canvasH = container.clientHeight;
    renderer.setSize(canvasW, canvasH);
    container.appendChild(renderer.domElement);

    // Poster shape
    const posterShape = new THREE.Shape();
    roundedRect(posterShape, 0, 0, POSTER.w, POSTER.h, 3);
    const posterGeometry = new THREE.ShapeGeometry(posterShape);

    // Group
    const startingY = -POSTER.h - POSTER.padding;
    const assetGroup = new THREE.Group();
    assetGroup.position.y = startingY;
    assetGroup.position.x =
      -(POSTER.w * POSTER.cols + POSTER.padding * (POSTER.cols - 1)) / 2;
    scene.add(assetGroup);

    // Camera
    const camera = new THREE.PerspectiveCamera(
      75,
      (canvasW / canvasH) * 0.5,
      0.01,
      1000,
    );
    camera.rotation.x = 0.6;
    camera.position.z = 100;
    camera.position.y = POSTER.h * 1.5;

    // Light
    const spotLight = new THREE.PointLight(0xffffff, 2500, 500);
    spotLight.position.set(0, POSTER.h * 1.5, 50);
    scene.add(spotLight);

    // Ambient for subtle visibility on edges
    const ambient = new THREE.AmbientLight(0x4444ff, 0.3);
    scene.add(ambient);

    // Create posters
    const posterRows: THREE.Group[] = [];
    const totalPosters = POSTER.cols * POSTER.rows;

    let x = 0;
    let y = 0;
    let rowGroup: THREE.Group | null = null;

    for (let i = 0; i < totalPosters; i++) {
      if (i % POSTER.cols === 0) {
        y += POSTER.h + POSTER.padding;
        x = 0;
        rowGroup = new THREE.Group();
        rowGroup.position.y = y;
        assetGroup.add(rowGroup);
        posterRows.push(rowGroup);
      } else {
        x += POSTER.w + POSTER.padding;
      }

      const texture = createPosterTexture(i);
      const material = new THREE.MeshStandardMaterial({
        color: 0xffffff,
        map: texture,
      });

      const poster = new THREE.Mesh(posterGeometry, material);
      poster.position.x = x;
      rowGroup!.add(poster);
    }

    // Animation
    let assetGroupY = startingY;
    let scrolling = false;
    let scrollTimeout: ReturnType<typeof setTimeout>;
    let frameCount = 1;
    let animId: number;

    const scrollPosters = (moveY: number) => {
      if (assetGroupY >= 0) {
        // Loop rows
        const lastY =
          POSTER.h * posterRows.length +
          POSTER.padding * (posterRows.length - 1);
        for (const row of posterRows) {
          if (row.position.y >= lastY) {
            row.position.y = -startingY;
          } else {
            row.position.y += -startingY;
          }
        }
        assetGroupY = startingY;
      } else {
        assetGroupY += moveY;
      }
    };

    const onWheel = (e: WheelEvent) => {
      clearTimeout(scrollTimeout);
      scrolling = true;
      scrollPosters(Math.abs(e.deltaY) * 0.05);
      scrollTimeout = setTimeout(() => {
        scrolling = false;
      }, 50);
    };

    container.addEventListener("wheel", onWheel);

    const animate = () => {
      animId = requestAnimationFrame(animate);
      if (frameCount % 2 === 0) {
        if (!scrolling) {
          scrollPosters(0.15);
        }
        frameCount = 1;
        assetGroup.position.y = assetGroupY;
        renderer.render(scene, camera);
      } else {
        frameCount++;
      }
    };

    animate();

    // Resize
    const onResize = () => {
      const w = container.clientWidth;
      const h = container.clientHeight;
      camera.aspect = (w / h) * 0.5;
      camera.updateProjectionMatrix();
      renderer.setSize(w, h);
    };
    window.addEventListener("resize", onResize);

    cleanupRef.current = () => {
      cancelAnimationFrame(animId);
      container.removeEventListener("wheel", onWheel);
      window.removeEventListener("resize", onResize);
      renderer.dispose();
      posterGeometry.dispose();
      if (renderer.domElement.parentNode) {
        renderer.domElement.parentNode.removeChild(renderer.domElement);
      }
    };

    return () => {
      cleanupRef.current?.();
    };
  }, []);

  return (
    <div ref={containerRef} className="absolute inset-0 overflow-hidden" />
  );
}
