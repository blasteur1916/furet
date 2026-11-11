import { useEffect } from "react";
import { Ile } from "./ile/Ile";

export default function App() {
  // Le menu contextuel de la WebView (recharger, inspecter…) n'a rien à faire sur l'île ; il reste en développement.
  useEffect(() => {
    if (!import.meta.env.PROD) return;
    const bloquer = (e: MouseEvent) => e.preventDefault();
    window.addEventListener("contextmenu", bloquer);
    return () => window.removeEventListener("contextmenu", bloquer);
  }, []);

  return <Ile />;
}
