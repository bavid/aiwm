# Produktvision

## Einzeiler

Eine lokale Kommandozentrale für die eigene AI-Workstation: Du sagst *was* du tun
willst, das Tool kümmert sich um Modelle, Runtimes und GPU-Ressourcen.

## Problem

Lokale AI besteht heute aus vielen Einzelwerkzeugen (Ollama, LM Studio, ComfyUI,
Agent-CLIs, Download-Tools, Modell-Websites). Der Nutzer trägt die Integrationslast:
Formate, Ordner, VRAM-Budgets, Versionsdrift, doppelte Downloads. Tools, die das
lösen wollen (z. B. Stability Matrix), exponieren stattdessen *noch mehr*
technische Parameter.

## Zielbild

```
PC einschalten → "AI Workstation" öffnen

┌───────────────────────────────┐
│  GPU  12.4 / 16 GB   RAM 21/32 │
│  Aktueller Job: —              │
│                               │
│  Was möchtest du tun?         │
│   [ Coding ] [ Bild ]         │
│   [ Video ]  [ Enhance ]      │
└───────────────────────────────┘
```

Der Rest passiert automatisch: passende Runtime starten, passendes Modell laden,
VRAM freiräumen, Job ausführen, Ergebnis zeigen.

## Leitprinzipien

1. **Capability vor Technik.** Die Oberfläche fragt „was", nicht „welches Modell".
2. **Local-first, Cloud-optional.** Der Kern funktioniert offline und ohne Accounts.
   Cloud ist ein Schalter, keine Grundlage.
3. **Automatik mit Transparenz.** Das Tool entscheidet — und zeigt in einem Satz,
   *warum* („Modell B: bestes Qualität/Speed-Verhältnis in deinem VRAM-Budget").
   Jede Entscheidung ist überschreibbar.
4. **Fehler in Klartext.** Kein `CUDA error 0x...`, sondern „Modell braucht 11.4 GB,
   frei sind 5.2 GB" plus Handlungsoptionen.
5. **Wenige, kuratierte Pfade.** Eine getestete Runtime-Version, feste Pipelines,
   bekannte Modellquellen. Kein frei konfigurierbares Python-Environment.
6. **Datenhoheit.** Private Fotos, Code, Repositories, Prompts, Agent-Memory
   bleiben lokal, es sei denn der Nutzer erlaubt pro Aktion explizit etwas anderes.
7. **Klein und wartbar.** Viele kleine Module mit klaren Schnittstellen. Kein
   Enterprise-Stack ohne konkreten Anlass.

## Zielnutzer

Eine Person (der Projektinhaber): technisch versiert, will aber im Alltag nicht
Modell-Logistik betreiben. Single-User, eine Workstation, lokal.

## Nicht-Ziele

- **Kein** Reimplementieren von Ollama/ComfyUI/llama.cpp — nur Orchestrierung.
- **Kein** Stability-Matrix-Klon mit noch mehr Reglern.
- **Kein** Multi-User-/Team-/SaaS-Produkt.
- **Kein** Cloud-Zwang, keine Telemetrie, kein Account.
- **Kein** Kubernetes, keine Microservices, kein Message-Broker ohne Not.
- **Kein** generischer Workflow-Editor (das ist ComfyUI selbst).
- **Kein** Modell-Trainer / Fine-Tuning-Tool (zunächst).

## Erfolgskriterien

- Workstation vom Netz trennen → Coding-Agent, Bildgenerierung, Enhancement,
  Video laufen weiter.
- Ein neues Coding-Modell einsatzbereit in < 5 Minuten, ohne eine Website zu öffnen.
- Wechsel „Coding → Bild generieren → zurück zu Coding" ohne manuelles
  VRAM-Management und ohne Absturz.
- Startbildschirm zeigt maximal: GPU/RAM, aktueller Job, vier Capability-Buttons.
- Der Projektinhaber empfindet es als *einfacher* als Ollama + ComfyUI einzeln.
