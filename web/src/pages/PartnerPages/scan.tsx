import { useCallback, useEffect, useRef, useState } from "react"
import jsQR from "jsqr"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../../components/ui/card"
import { Button } from "../../components/ui/button"
import { Input } from "../../components/ui/input"
import { Watermark } from "../../components/Watermark"
import { api, ApiError } from "../../api"
import { getUser } from "../../auth"

type DetectedBarcode = { rawValue: string; format: string }
type BarcodeDetectorLike = {
  detect: (source: CanvasImageSource) => Promise<DetectedBarcode[]>
}
type BarcodeDetectorCtor = {
  new(options?: { formats: string[] }): BarcodeDetectorLike
}
declare global {
  interface Window {
    BarcodeDetector?: BarcodeDetectorCtor
  }
}

type Status =
  | "idle"
  | "starting"
  | "scanning"
  | "scanned"
  | "paying"
  | "paid"
  | "error"

const SCAN_INTERVAL_MS = 250

export default function PartnerScanPage() {
  const videoRef = useRef<HTMLVideoElement>(null)
  const streamRef = useRef<MediaStream | null>(null)
  const detectorRef = useRef<BarcodeDetectorLike | null>(null)
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const loopRef = useRef<number | null>(null)

  const [status, setStatus] = useState<Status>("idle")
  const [error, setError] = useState<string | null>(null)
  const [result, setResult] = useState<string | null>(null)
  const [amount, setAmount] = useState("")
  const [manual, setManual] = useState("")

  const stopCamera = useCallback(() => {
    if (loopRef.current !== null) {
      window.clearInterval(loopRef.current)
      loopRef.current = null
    }
    streamRef.current?.getTracks().forEach((track) => track.stop())
    streamRef.current = null
    if (videoRef.current) videoRef.current.srcObject = null
  }, [])

  const handleDetected = useCallback(
    (value: string) => {
      stopCamera()
      setResult(value)
      setAmount("")
      setError(null)
      setStatus("scanned")
    },
    [stopCamera],
  )

  /** Analyse une frame : BarcodeDetector natif si dispo, sinon jsQR. */
  const scanTick = useCallback(async () => {
    const video = videoRef.current
    if (!video || video.readyState < 2) return

    const detector = detectorRef.current
    if (detector) {
      try {
        const codes = await detector.detect(video)
        if (codes.length > 0 && codes[0].rawValue) {
          handleDetected(codes[0].rawValue)
        }
      } catch {
        /* image illisible sur cette frame */
      }
      return
    }

    const width = video.videoWidth
    const height = video.videoHeight
    if (!width || !height) return

    let canvas = canvasRef.current
    if (!canvas) {
      canvas = document.createElement("canvas")
      canvasRef.current = canvas
    }
    canvas.width = width
    canvas.height = height
    const ctx = canvas.getContext("2d", { willReadFrequently: true })
    if (!ctx) return
    ctx.drawImage(video, 0, 0, width, height)
    const image = ctx.getImageData(0, 0, width, height)
    const code = jsQR(image.data, width, height, {
      inversionAttempts: "dontInvert",
    })
    if (code?.data) handleDetected(code.data)
  }, [handleDetected])

  const startCamera = useCallback(async () => {
    setError(null)
    setResult(null)

    if (!window.isSecureContext) {
      setError(
        "L'accès à la caméra nécessite une connexion sécurisée (HTTPS ou localhost).",
      )
      setStatus("error")
      return
    }
    if (!navigator.mediaDevices?.getUserMedia) {
      setError("Ce navigateur ne permet pas l'accès à la caméra.")
      setStatus("error")
      return
    }

    setStatus("starting")
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        video: { facingMode: { ideal: "environment" } },
        audio: false,
      })
      streamRef.current = stream

      const video = videoRef.current
      if (!video) {
        stopCamera()
        return
      }
      video.srcObject = stream
      await video.play()

      detectorRef.current = window.BarcodeDetector
        ? new window.BarcodeDetector({ formats: ["qr_code"] })
        : null
      setStatus("scanning")

      loopRef.current = window.setInterval(() => {
        void scanTick()
      }, SCAN_INTERVAL_MS)
    } catch (err) {
      if (err instanceof DOMException && err.name === "NotAllowedError") {
        setError("Accès à la caméra refusé.")
      } else if (err instanceof DOMException && err.name === "NotFoundError") {
        setError("Aucune caméra détectée sur cet appareil.")
      } else {
        setError("Impossible de démarrer la caméra.")
      }
      setStatus("error")
      stopCamera()
    }
  }, [scanTick, stopCamera])

  useEffect(() => stopCamera, [stopCamera])

  function reset() {
    stopCamera()
    setStatus("idle")
    setError(null)
    setResult(null)
    setAmount("")
    setManual("")
  }

  function submitManual(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault()
    if (manual.trim()) handleDetected(manual.trim())
  }

  async function confirmPayment(event: React.FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const value = Number(amount.replace(",", "."))
    if (!Number.isFinite(value) || value <= 0) {
      setError("Montant invalide.")
      return
    }
    const partner = getUser()
    if (!partner) {
      setError("Vous devez être connecté en tant que partenaire.")
      return
    }
    if (!result) {
      setError("Aucun QR code détecté.")
      return
    }

    setError(null)
    setStatus("paying")
    try {
      await api("/process/payment", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          qr_token: result,
          partner_id: partner.id,
          amount: value,
        }),
      })
      setStatus("paid")
    } catch (err) {
      setError(
        err instanceof ApiError
          ? err.message
          : "Impossible de valider le paiement.",
      )
      setStatus("scanned")
    }
  }

  const parsedAmount = Number(amount.replace(",", "."))
  const formattedAmount =
    Number.isFinite(parsedAmount) && parsedAmount > 0
      ? parsedAmount.toLocaleString("fr-FR", {
        style: "currency",
        currency: "EUR",
      })
      : null

  const showScanner =
    status === "idle" ||
    status === "starting" ||
    status === "scanning" ||
    status === "error"

  return (
    <Watermark text="PAIEMENT RÉEL — DÉBIT EFFECTIF DU SOLDE">
      <main className="mx-auto flex w-full max-w-xl flex-col gap-6 p-6 md:p-10">
        <div>
          <h1 className="text-2xl font-semibold">Encaisser un paiement</h1>
          <p className="text-sm text-muted-foreground">
            Scannez le QR code présenté par le client pour initier le paiement.
          </p>
        </div>

        <Card>
          <CardHeader>
            <CardTitle>Scanner le QR code</CardTitle>
            <CardDescription>
              Placez le QR code du client dans le cadre.
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-col gap-4">
            {showScanner && (
              <>
                <div className="relative aspect-square w-full overflow-hidden rounded-lg border bg-muted">
                  <video
                    ref={videoRef}
                    playsInline
                    muted
                    className="h-full w-full object-cover"
                  />
                  {status === "scanning" ? (
                    <div className="pointer-events-none absolute inset-6 rounded-lg border-2 border-primary/70" />
                  ) : (
                    <div className="absolute inset-0 flex items-center justify-center p-6 text-center text-sm text-muted-foreground">
                      {status === "starting"
                        ? "Démarrage de la caméra…"
                        : "Caméra inactive"}
                    </div>
                  )}
                </div>

                <div className="flex gap-2">
                  {status === "scanning" ? (
                    <Button type="button" variant="outline" onClick={reset}>
                      Arrêter
                    </Button>
                  ) : (
                    <Button
                      type="button"
                      onClick={startCamera}
                      disabled={status === "starting"}
                    >
                      {status === "starting"
                        ? "Démarrage…"
                        : "Démarrer la caméra"}
                    </Button>
                  )}
                </div>

                {error && <p className="text-sm text-destructive">{error}</p>}

                <form
                  className="flex flex-col gap-2 border-t pt-4"
                  onSubmit={submitManual}
                >
                  <label htmlFor="manual" className="text-sm font-medium">
                    Saisie manuelle
                  </label>
                  <div className="flex gap-2">
                    <Input
                      id="manual"
                      value={manual}
                      onChange={(e) => setManual(e.target.value)}
                      placeholder="Contenu du QR code"
                    />
                    <Button
                      type="submit"
                      variant="outline"
                      disabled={!manual.trim()}
                    >
                      Valider
                    </Button>
                  </div>
                </form>
              </>
            )}

            {(status === "scanned" || status === "paying") && result && (
              <form className="flex flex-col gap-4" onSubmit={confirmPayment}>
                <div className="rounded-lg border bg-muted/40 p-3">
                  <p className="text-xs uppercase text-muted-foreground">
                    QR code détecté
                  </p>
                  <p className="break-all font-medium">{result}</p>
                </div>

                <div className="flex flex-col gap-2">
                  <label htmlFor="amount" className="text-sm font-medium">
                    Montant à encaisser
                  </label>
                  <Input
                    id="amount"
                    inputMode="decimal"
                    placeholder="0,00"
                    value={amount}
                    onChange={(e) => setAmount(e.target.value)}
                    disabled={status === "paying"}
                    required
                  />
                </div>

                {error && <p className="text-sm text-destructive">{error}</p>}

                <div className="flex gap-2">
                  <Button type="submit" disabled={status === "paying"}>
                    {status === "paying" ? "Validation…" : "Valider le paiement"}
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    disabled={status === "paying"}
                    onClick={reset}
                  >
                    Annuler
                  </Button>
                </div>
              </form>
            )}

            {status === "paid" && (
              <div className="flex flex-col items-center gap-4 py-4 text-center">
                <div className="flex size-12 items-center justify-center rounded-full bg-positive/10 text-positive">
                  <svg
                    viewBox="0 0 24 24"
                    fill="none"
                    className="size-6"
                    stroke="currentColor"
                    strokeWidth={2}
                  >
                    <path
                      strokeLinecap="round"
                      strokeLinejoin="round"
                      d="m5 13 4 4L19 7"
                    />
                  </svg>
                </div>
                <div>
                  <p className="text-lg font-semibold">Paiement validé</p>
                  {formattedAmount && (
                    <p className="text-sm text-muted-foreground">
                      {formattedAmount} encaissés.
                    </p>
                  )}
                </div>
                <Button type="button" onClick={reset}>
                  Nouveau scan
                </Button>
              </div>
            )}
          </CardContent>
        </Card>
      </main>
    </Watermark>
  )
}
