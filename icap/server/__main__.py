#!/bin/env python
# -*- coding: utf8 -*-

import sys
import random
import SocketServer

from pyicap import *

class ThreadingSimpleServer(SocketServer.ThreadingMixIn, ICAPServer):
    pass

class ICAPHandler(BaseICAPRequestHandler):

    def request_OPTIONS(self):
        self.set_icap_response(200)
        self.set_icap_header('Methods', 'RESPMOD')
        self.set_icap_header('Preview', '0')
        self.send_headers(True)

    def request_RESPMOD(self):
        self._inspect()
        self.no_adaptation_required()

    def request_REQMOD(self):
        self._inspect()
        self.no_adaptation_required()

    def _inspect(self):
        print(self.encapsulated)
        while not self.ieof:
            chunk = self.read_chunk()
            if not chunk:
                break
            print('CHUNK >> ' + str(chunk))

        sys.stdout.flush()

port = 13440

server = ThreadingSimpleServer(('', port), ICAPHandler)
try:
    while 1:
        server.handle_request()
except KeyboardInterrupt:
    print("Finished")
