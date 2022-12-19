FROM python:2.7

ARG TARGET_DIR=/icap

RUN mkdir $TARGET_DIR
WORKDIR $TARGET_DIR
ADD . $TARGET_DIR
RUN pip install -r $TARGET_DIR/requirements.txt

ENTRYPOINT python -m server
