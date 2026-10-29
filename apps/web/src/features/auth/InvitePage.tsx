import { useEffect, useRef } from "react";
import { Link, Navigate, useNavigate, useParams } from "react-router";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { api, errorMessage } from "@/lib/api";
import { useMe } from "@/lib/queries";
import { Loading } from "@/components/ui";
import { Wordmark } from "@/app/Layout";

export function InvitePage() {
  const { token = "" } = useParams();
  const { data: me, isLoading } = useMe();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const accept = useMutation({
    mutationFn: () => api.post("/invitations/accept", { token }),
    onSuccess: async () => {
      await qc.invalidateQueries();
      navigate("/");
    },
  });
  const started = useRef(false);
  useEffect(() => {
    if (me && !started.current) {
      started.current = true;
      accept.mutate();
    }
  }, [me, accept]);

  if (isLoading) return <Loading />;
  if (!me) return <Navigate to={`/login?return_to=${encodeURIComponent(`/invite/${token}`)}`} replace />;

  return (
    <div className="min-h-dvh px-4 py-8 md:px-12 md:py-12">
      <Wordmark />
      <div className="mt-16 max-w-xl">
        {accept.isError ? (
          <>
            <h1 className="text-title font-semibold">This invitation did not work</h1>
            <p className="mt-4 text-lg">{errorMessage(accept.error)}</p>
            <p className="mt-6">
              <Link className="underline underline-offset-4" to="/setup">
                Continue to your account
              </Link>
            </p>
          </>
        ) : (
          <Loading label="Joining the organization" />
        )}
      </div>
    </div>
  );
}
