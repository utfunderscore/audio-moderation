set -a
source ../.env
set +a

cd ui
AWS_PROFILE=admin npm run deploy
