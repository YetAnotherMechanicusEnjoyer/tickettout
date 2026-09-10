import { useEffect, useState } from "react";
import BankCard from "../../components/FlipCard";
import { Watermark } from "../../components/Watermark";
import { api, ApiError } from "../../api";
import { getToken } from "../../auth";

type TokenBody = {
  qr_token: string;
};

export default function EmployeePaymentPage() {
  const [qrToken, setQrToken] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  function generateToken() {
    const token = getToken();
    if (!token) {
      setError("Vous devez être connecté pour générer un QR code.");
      setLoading(false);
      return;
    }

    setLoading(true);
    setError(null);
    api<TokenBody>("/begin/payment", {
      method: "POST",
      headers: { Authorization: `Bearer ${token}` },
    })
      .then((data) => setQrToken(data.qr_token))
      .catch((err) => {
        setError(
          err instanceof ApiError
            ? err.message
            : "Impossible de générer le QR code.",
        );
      })
      .finally(() => setLoading(false));
  }

  useEffect(() => {
    generateToken();
  }, []);

  return (
    <Watermark text="QR DE PAIEMENT RÉEL">
      <main className="mt-20 flex flex-col items-center justify-center gap-4 px-4">
        {loading && (
          <p className="text-sm text-muted-foreground">
            Génération du QR code…
          </p>
        )}
        {error && (
          <div className="flex flex-col items-center gap-2 text-center">
            <p className="text-sm text-destructive">{error}</p>
            <button
              type="button"
              onClick={generateToken}
              className="text-sm font-medium text-primary underline underline-offset-4"
            >
              Réessayer
            </button>
          </div>
        )}
        {!loading && !error && qrToken && (
          <>
            <BankCard paymentUrl={qrToken} />
            <button
              type="button"
              onClick={generateToken}
              className="text-sm font-medium text-primary underline underline-offset-4"
            >
              Générer un nouveau QR code
            </button>
          </>
        )}
      </main>
    </Watermark>
  );
}
