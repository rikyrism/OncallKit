{{- define "oncallkit.name" -}}
{{- .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "oncallkit.fullname" -}}
{{- if contains .Chart.Name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "oncallkit.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" }}
app.kubernetes.io/name: {{ include "oncallkit.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{/* Selector labels for one component: api, worker, web or postgresql. */}}
{{- define "oncallkit.selectorLabels" -}}
app.kubernetes.io/name: {{ include "oncallkit.name" .root }}
app.kubernetes.io/instance: {{ .root.Release.Name }}
app.kubernetes.io/component: {{ .component }}
{{- end -}}

{{- define "oncallkit.image" -}}
{{- $img := index .root.Values.image .component -}}
{{- printf "%s/%s:%s" .root.Values.image.registry $img.repository (.root.Values.image.tag | default .root.Chart.AppVersion) -}}
{{- end -}}

{{- define "oncallkit.secretName" -}}
{{- .Values.existingSecret | default (include "oncallkit.fullname" .) -}}
{{- end -}}

{{- define "oncallkit.postgresqlName" -}}
{{- printf "%s-postgresql" (include "oncallkit.fullname" .) | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{/* Fail early on configuration the server would refuse at start-up anyway. */}}
{{- define "oncallkit.validate" -}}
{{- if not .Values.publicUrl -}}
{{- fail "set publicUrl to the address people open OncallKit at, e.g. --set publicUrl=https://oncall.example.com" -}}
{{- end -}}
{{- if and (eq .Values.appEnv "production") (not (hasPrefix "https://" .Values.publicUrl)) -}}
{{- fail "publicUrl must start with https:// when appEnv is production" -}}
{{- end -}}
{{- if and (eq .Values.appEnv "production") .Values.auth.devLogin -}}
{{- fail "auth.devLogin cannot be enabled when appEnv is production" -}}
{{- end -}}
{{- if and (not .Values.existingSecret) (not .Values.auth.devLogin) (not (and .Values.google.clientId .Values.google.clientSecret)) -}}
{{- fail "nobody could sign in: set google.clientId and google.clientSecret (or provide them in existingSecret)" -}}
{{- end -}}
{{- if and (not .Values.postgresql.enabled) (not .Values.database.url) (not .Values.existingSecret) -}}
{{- fail "set database.url (or DATABASE_URL in existingSecret), or postgresql.enabled=true for a bundled database" -}}
{{- end -}}
{{- end -}}

{{/* Environment shared by the API and the worker. */}}
{{- define "oncallkit.env" -}}
- name: APP_ENV
  value: {{ .Values.appEnv | quote }}
- name: PUBLIC_URL
  value: {{ .Values.publicUrl | quote }}
- name: BIND_ADDR
  value: "0.0.0.0:8080"
- name: TRUST_PROXY_HEADERS
  value: "true"
- name: LOG_JSON
  value: {{ .Values.logJson | quote }}
- name: DATABASE_MAX_CONNECTIONS
  value: {{ .Values.database.maxConnections | quote }}
- name: SESSION_TTL_HOURS
  value: {{ .Values.auth.sessionTtlHours | quote }}
- name: AUTH_DEV_LOGIN
  value: {{ .Values.auth.devLogin | quote }}
{{- with .Values.auth.registrationMode }}
- name: REGISTRATION_MODE
  value: {{ . | quote }}
{{- end }}
{{- with .Values.auth.allowedEmailDomains }}
- name: ALLOWED_EMAIL_DOMAINS
  value: {{ join "," . | quote }}
{{- end }}
{{- with .Values.auth.orgCreation }}
- name: ORG_CREATION
  value: {{ . | quote }}
{{- end }}
{{- range $key := list "ENCRYPTION_KEY" "GOOGLE_CLIENT_ID" "GOOGLE_CLIENT_SECRET" "METRICS_TOKEN" }}
- name: {{ $key }}
  valueFrom:
    secretKeyRef:
      name: {{ include "oncallkit.secretName" $ }}
      key: {{ $key }}
      optional: true
{{- end }}
{{- if .Values.postgresql.enabled }}
- name: POSTGRES_PASSWORD
  valueFrom:
    secretKeyRef:
      name: {{ include "oncallkit.postgresqlName" . }}
      key: POSTGRES_PASSWORD
- name: DATABASE_URL
  value: "postgres://oncallkit:$(POSTGRES_PASSWORD)@{{ include "oncallkit.postgresqlName" . }}:5432/oncallkit"
{{- else }}
- name: DATABASE_URL
  valueFrom:
    secretKeyRef:
      name: {{ include "oncallkit.secretName" . }}
      key: DATABASE_URL
{{- end }}
{{- end -}}

{{- define "oncallkit.podSecurity" -}}
securityContext:
  {{- toYaml .Values.podSecurityContext | nindent 2 }}
automountServiceAccountToken: false
{{- with .Values.imagePullSecrets }}
imagePullSecrets:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- end -}}
