FROM gcr.io/oss-fuzz-base/base-builder-python@sha256:34a0a1575c6eae0ff2a1e98ea62c6007f62432a56efb46484b1c5b82ace5a05a

WORKDIR /src
COPY --chown=65532:65532 . /src
USER 65532:65532

# Legacy CFLite compile marker, not a production image or a PR Python toolchain.
RUN python -m compileall /src
