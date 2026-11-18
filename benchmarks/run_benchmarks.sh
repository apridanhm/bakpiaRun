#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPOSE_FILE="${SCRIPT_DIR}/docker-compose.benchmark.yaml"
RESULTS_DIR="${SCRIPT_DIR}/results/$(date +%Y%m%d_%H%M%S)"
mkdir -p "${RESULTS_DIR}"

echo "================================================================="
echo " BakpiaRun Thesis Benchmark Suite (k6 + cgroups v2 Evaluation)"
echo " Output Directory: ${RESULTS_DIR}"
echo "================================================================="

if ! command -v k6 >/dev/null 2>&1; then
  echo "❌ 'k6' belum terpasang di mesin ini."
  echo "   Install k6 terlebih dahulu (contoh macOS: 'brew install k6' atau Ubuntu: snap/apt grafana k6)."
  exit 1
fi

TARGETS=(
  "bakpiarun|http://127.0.0.1:8081|bench-bakpiarun"
  "phpfpm|http://127.0.0.1:8082|bench-phpfpm"
  "frankenphp|http://127.0.0.1:8083|bench-frankenphp"
)

SCENARIOS=(
  "scenario1_baseline_light.js"
  "scenario2_mixed_workload.js"
  "scenario3_anti_oom_stress.js"
)

REPORT_MD="${RESULTS_DIR}/thesis_bab4_summary.md"
cat <<EOF > "${REPORT_MD}"
# Hasil Evaluasi Komparatif Bab 4 Tesis
Tanggal Eksekusi: $(date -u +"%Y-%m-%d %H:%M:%S UTC")
Batas Kontainer: \`2.0 vCPU\`, \`384 MB RAM (No Swap)\`, \`20 Workers/Threads\`

| Skenario | Runtime | Status Kontainer Akhir | Insiden OOMKilled | Exit Code | Memori Akhir | Catatan Tambahan |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
EOF

for scen in "${SCENARIOS[@]}"; do
  SCEN_NAME="${scen%.js}"
  echo ""
  echo "#################################################################"
  echo " MENJALANKAN SKENARIO: ${SCEN_NAME}"
  echo "#################################################################"

  for entry in "${TARGETS[@]}"; do
    IFS='|' read -r NAME URL CONTAINER <<< "${entry}"
    echo ""
    echo ">>> Merestart kontainer bersih untuk target: ${NAME} (${CONTAINER})..."
    docker compose -f "${COMPOSE_FILE}" up -d --force-recreate "${NAME}"
    sleep 5

    SUMMARY_JSON="${RESULTS_DIR}/${SCEN_NAME}_${NAME}.json"
    LOG_TXT="${RESULTS_DIR}/${SCEN_NAME}_${NAME}.txt"

    echo ">>> Menjalankan k6 (${SCEN_NAME}) -> ${URL} ..."
    k6 run \
      -e BASE_URL="${URL}" \
      --summary-export="${SUMMARY_JSON}" \
      "${SCRIPT_DIR}/k6/${scen}" | tee "${LOG_TXT}" || true

    # Periksa status OOMKilled & memori kontainer dari Docker Engine
    STATE_STATUS=$(docker inspect -f '{{.State.Status}}' "${CONTAINER}" 2>/dev/null || echo "unknown")
    OOM_KILLED=$(docker inspect -f '{{.State.OOMKilled}}' "${CONTAINER}" 2>/dev/null || echo "unknown")
    EXIT_CODE=$(docker inspect -f '{{.State.ExitCode}}' "${CONTAINER}" 2>/dev/null || echo "-")
    MEM_USAGE=$(docker stats --no-stream --format "{{.MemUsage}}" "${CONTAINER}" 2>/dev/null || echo "-")

    EXTRA_NOTE="-"
    if [[ "${NAME}" == "bakpiarun" && "${STATE_STATUS}" == "running" ]]; then
      HEALTH_OUT="${RESULTS_DIR}/${SCEN_NAME}_bakpiarun_health.json"
      curl -s -H "X-Admin-Token: bench-secret-token" "${URL}/health" > "${HEALTH_OUT}" || true
      RECYCLES=$(python3 -c "import json; d=json.load(open('${HEALTH_OUT}')); print(d.get('anti_oom',{}).get('total_worker_recycles', 0))" 2>/dev/null || echo "0")
      EXTRA_NOTE="Graceful Recycles: ${RECYCLES}"
    fi

    echo "| ${SCEN_NAME} | **${NAME}** | ${STATE_STATUS} | **${OOM_KILLED}** | ${EXIT_CODE} | ${MEM_USAGE} | ${EXTRA_NOTE} |" >> "${REPORT_MD}"
  done
done

echo ""
echo "✅ Seluruh rangkaian benchmark selesai!"
echo "📄 Laporan ringkasan Bab 4 tersimpan di: ${REPORT_MD}"
