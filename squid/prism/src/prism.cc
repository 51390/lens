#include <cstring>
#include <dlfcn.h>
#include <iostream>
#include <libecap/common/autoconf.h>
#include <libecap/common/registry.h>
#include <libecap/common/errors.h>
#include <libecap/common/message.h>
#include <libecap/common/header.h>
#include <libecap/common/names.h>
#include <libecap/common/named_values.h>
#include <libecap/host/host.h>
#include <libecap/adapter/service.h>
#include <libecap/adapter/xaction.h>
#include <libecap/host/xaction.h>
#include <pthread.h>

namespace Adapter { // not required, but adds clarity

using libecap::size_type;

libecap::Name headerContentEncoding("Content-Encoding", libecap::Name::NextId());

typedef struct {
    size_t size;
    const void* bytes;
} Chunk;


class Service: public libecap::adapter::Service {
	public:
        Service();
		// About
		virtual std::string uri() const; // unique across all vendors
		virtual std::string tag() const; // changes with version and config
		virtual void describe(std::ostream &os) const; // free-format info

		// Configuration
		virtual void configure(const libecap::Options &cfg);
		virtual void reconfigure(const libecap::Options &cfg);
		void setOne(const libecap::Name &name, const libecap::Area &valArea);

		// Lifecycle
		virtual void start(); // expect makeXaction() calls
		virtual void stop(); // no more makeXaction() calls until start()
		virtual void retire(); // no more makeXaction() calls

		// Scope (XXX: this may be changed to look at the whole header)
		virtual bool wantsUrl(const char *url) const;

		// Work
		virtual MadeXactionPointer makeXaction(libecap::host::Xaction *hostx);

        void (*init)();
        void (*transfer)(int, const void*, size_t, const char*);
        void (*commit)(int, const char*, const char*);
        void (*header)(int, const char*, const char*, const char*);
        void (*content_done)(int);
        Chunk (*get_content)(int);
    private:
        void * module_;
        std::string analyzerPath;
};


// Calls Service::setOne() for each host-provided configuration option.
// See Service::configure().
class Cfgtor: public libecap::NamedValueVisitor {
	public:
		Cfgtor(Service &aSvc): svc(aSvc) {}
		virtual void visit(const libecap::Name &name, const libecap::Area &value) {
			svc.setOne(name, value);
		}
		Service &svc;
};

class HeaderVisitor: public libecap::NamedValueVisitor {
    public:
        HeaderVisitor(libecap::shared_ptr<const Service> aSvc, int anId, const char* anUri): svc(aSvc), id(anId), requestUri(anUri) {}
        virtual void visit(const libecap::Name &name, const libecap::Area &value) {
            svc->header(id, name.image().c_str(), value.start, requestUri);
        }

    private:
        libecap::shared_ptr<const Service> svc;
        int id;
        const char* requestUri;
};


class Xaction: public libecap::adapter::Xaction {
	public:
		Xaction(libecap::shared_ptr<Service> s, libecap::host::Xaction *x);
		virtual ~Xaction();

		// meta-information for the host transaction
		virtual const libecap::Area option(const libecap::Name &name) const;
		virtual void visitEachOption(libecap::NamedValueVisitor &visitor) const;

		// lifecycle
		virtual void start();
		virtual void stop();

		// adapted body transmission control
		virtual void abDiscard();
		virtual void abMake();
		virtual void abMakeMore();
		virtual void abStopMaking();

		// adapted body content extraction and consumption
		virtual libecap::Area abContent(size_type offset, size_type size);
		virtual void abContentShift(size_type size);

		// virgin body state notification
		virtual void noteVbContentDone(bool atEnd);
		virtual void noteVbContentAvailable();

	protected:
		void adaptContent(std::string &chunk) const; // converts vb to ab
		void stopVb(); // stops receiving vb (if we are receiving it)
		libecap::host::Xaction *lastHostCall(); // clears hostx

	private:
        void _processBuffers();

		libecap::shared_ptr<const Service> service;
        libecap::shared_ptr<libecap::Message>  adapted;
		libecap::host::Xaction *hostx; // Host transaction rep
        bool gzip = false;
                                
        int id = 0;
        int contentLength = 0;
        char* requestUri = 0;

		typedef enum { opUndecided, opOn, opComplete, opNever } OperationState;
		OperationState receivingVb;
		OperationState sendingAb;
};

static const std::string CfgErrorPrefix =
	"Modifying Adapter: configuration error: ";

} // namespace Adapter

Adapter::Service::Service(): libecap::adapter::Service() {
}

std::string Adapter::Service::uri() const {
	return "ecap://e-cap.org/ecap/services/51390/prism";
}

std::string Adapter::Service::tag() const {
	return PACKAGE_VERSION;
}

void Adapter::Service::describe(std::ostream &os) const {
	os << "A modifying adapter from " << PACKAGE_NAME << " v" << PACKAGE_VERSION;
}

void Adapter::Service::configure(const libecap::Options &cfg) {
	Cfgtor cfgtor(*this);
	cfg.visitEachOption(cfgtor);
}

void Adapter::Service::reconfigure(const libecap::Options &cfg) {
	configure(cfg);
}

void Adapter::Service::setOne(const libecap::Name &key, const libecap::Area &val) {
	const std::string value = val.toString();
    const std::string name = key.image();
    if (key.assignedHostId()) {
		// skip host-standard options we do not know or care about
    } else if(name == "analyzerPath") {
        analyzerPath = value;
    } else
		throw libecap::TextException(CfgErrorPrefix +
			"unsupported configuration parameter: " + name);
}

void Adapter::Service::start() {
	libecap::adapter::Service::start();

    std::clog << "Prism starting";

    module_ = dlopen(analyzerPath.c_str(), RTLD_NOW | RTLD_GLOBAL);

    if(module_) {
        init = (void (*)())dlsym(module_, "init");
        init();

        transfer = (void (*)(int, const void*, size_t, const char*))dlsym(module_, "transfer");
        commit = (void (*)(int, const char*, const char*))dlsym(module_, "commit");
        header = (void (*)(int, const char*, const char*, const char*))dlsym(module_, "header");
        get_content = (Chunk (*)(int))dlsym(module_, "get_content");
        content_done = (void (*)(int))dlsym(module_, "content_done");
    }
}

void Adapter::Service::stop() {
	// custom code would go here, but this service does not have one
	libecap::adapter::Service::stop();
}

void Adapter::Service::retire() {
	// custom code would go here, but this service does not have one
	libecap::adapter::Service::stop();
}

bool Adapter::Service::wantsUrl(const char *url) const {
	return true; // no-op is applied to all messages
}

Adapter::Service::MadeXactionPointer
Adapter::Service::makeXaction(libecap::host::Xaction *hostx) {
	return Adapter::Service::MadeXactionPointer(
		new Adapter::Xaction(std::tr1::static_pointer_cast<Service>(self), hostx));
}

int counter = 0;

Adapter::Xaction::Xaction(libecap::shared_ptr<Service> aService,
	libecap::host::Xaction *x):
	service(aService),
	hostx(x),
	receivingVb(opUndecided), sendingAb(opUndecided) {
    id = counter++;
}

Adapter::Xaction::~Xaction() {
	if (libecap::host::Xaction *x = hostx) {
		hostx = 0;
		x->adaptationAborted();
	}

}

const libecap::Area Adapter::Xaction::option(const libecap::Name &) const {
	return libecap::Area(); // this transaction has no meta-information
}

void Adapter::Xaction::visitEachOption(libecap::NamedValueVisitor &) const {
	// this transaction has no meta-information to pass to the visitor
}


void Adapter::Xaction::start() {
	Must(hostx);
	if (hostx->virgin().body()) {
		receivingVb = opOn;
		hostx->vbMake(); // ask host to supply virgin body
	} else {
		// we are not interested in vb if there is not one
		receivingVb = opNever;
	}

	/* adapt message header */

	adapted = hostx->virgin().clone();
	Must(adapted != 0);

    if(adapted->header().hasAny(libecap::headerContentLength)) {
        libecap::Header::Value v = adapted->header().value(libecap::headerContentLength);
        contentLength = std::stoi(v.toString());
        // delete ContentLength header because we may change the length
        // unknown length may have performance implications for the host
        adapted->header().removeAny(libecap::headerContentLength);
    }

    /*
    if(adapted->header().hasAny(Adapter::headerContentEncoding)) {
        libecap::Header::Value v = adapted->header().value(Adapter::headerContentEncoding);
        if(v.toString() == "gzip") {
            //adapted->header().removeAny(Adapter::headerContentEncoding);
            service->header(id, "Content-Encoding", "gzip", "N/A");
        }
    }
    */


	// add a custom header
	static const libecap::Name name("X-Ecap");
	const libecap::Header::Value value =
		libecap::Area::FromTempString(libecap::MyHost().uri());
	adapted->header().add(name, value);

	if (!adapted->body()) {
		sendingAb = opNever; // there is nothing to send
		lastHostCall()->useAdapted(adapted);
	} else {
		hostx->useAdapted(adapted);
	}

}

void Adapter::Xaction::stop() {
	hostx = 0;
	// the caller will delete

    _processBuffers();

    if(requestUri) {
        free(requestUri);
    }
}

void Adapter::Xaction::_processBuffers() {
    if(adapted != 0 && adapted->header().hasAny(Adapter::headerContentEncoding)) {
        libecap::Header::Value v = adapted->header().value(Adapter::headerContentEncoding);
        service->commit(id, v.start, requestUri);
    } else {
        service->commit(id, "N/A", requestUri);
    }
}

void Adapter::Xaction::abDiscard()
{
	Must(sendingAb == opUndecided); // have not started yet
	sendingAb = opNever;
	// we do not need more vb if the host is not interested in ab
	stopVb();
}

void Adapter::Xaction::abMake()
{
	Must(sendingAb == opUndecided); // have not yet started or decided not to send
	Must(hostx->virgin().body()); // that is our only source of ab content

	// we are or were receiving vb
	Must(receivingVb == opOn || receivingVb == opComplete);
	
	sendingAb = opOn;
    hostx->noteAbContentAvailable();
}

void Adapter::Xaction::abMakeMore()
{
	Must(receivingVb == opOn); // a precondition for receiving more vb
	hostx->vbMakeMore();
}

void Adapter::Xaction::abStopMaking()
{
	sendingAb = opComplete;
	// we do not need more vb if the host is not interested in more ab
	stopVb();
}

libecap::Area Adapter::Xaction::abContent(size_type offset, size_type size) {
	Must(sendingAb == opOn || sendingAb == opComplete);
    Chunk c = service->get_content(id);

    if(c.size) {
        return libecap::Area::FromTempBuffer((const char*)c.bytes, c.size);
    } else {
        return libecap::Area();
    }
}

void Adapter::Xaction::abContentShift(size_type size) {
	Must(sendingAb == opOn || sendingAb == opComplete);
}

void Adapter::Xaction::noteVbContentDone(bool atEnd)
{
	Must(receivingVb == opOn);
	stopVb();
    service->content_done(id);
	if (sendingAb == opOn) {
		hostx->noteAbContentDone(atEnd);
		sendingAb = opComplete;
	}
}

void Adapter::Xaction::noteVbContentAvailable()
{
	Must(receivingVb == opOn);

    if(!requestUri) {
        // grab the request uri
        const libecap::Message& cause = hostx->cause();
        const libecap::RequestLine& rl = dynamic_cast<const libecap::RequestLine&>(cause.firstLine());
        const libecap::Area uri = rl.uri();
        requestUri = (char*)malloc(uri.size + 1);
        memset(requestUri, 0, uri.size + 1);
        memcpy(requestUri, uri.start, uri.size);

        HeaderVisitor hv(service, id, requestUri);
        adapted->header().visitEach(hv);

    }

	const libecap::Area vb = hostx->vbContent(0, libecap::nsize); // get all vb
    
	hostx->vbContentShift(vb.size); // we have a copy; do not need vb any more

    if (vb.size && vb.start) {
        char filename[1024];
        memset(filename, 0, sizeof(filename));
        snprintf(filename, sizeof(filename), "/tmp/vb-content-%d.log", id);
        FILE* f = fopen(filename, "a+");
        fwrite(vb.start, sizeof(char), vb.size, f);
        fclose(f);
        service->transfer(id, vb.start, vb.size, requestUri);
    }

	if (sendingAb == opOn)
		hostx->noteAbContentAvailable();
}

void Adapter::Xaction::adaptContent(std::string &chunk) const {
}

// tells the host that we are not interested in [more] vb
// if the host does not know that already
void Adapter::Xaction::stopVb() {
	if (receivingVb == opOn) {
		hostx->vbStopMaking(); // we will not call vbContent() any more
		receivingVb = opComplete;
	} else {
		// we already got the entire body or refused it earlier
		Must(receivingVb != opUndecided);
	}
}

// this method is used to make the last call to hostx transaction
// last call may delete adapter transaction if the host no longer needs it
// TODO: replace with hostx-independent "done" method
libecap::host::Xaction *Adapter::Xaction::lastHostCall() {
	libecap::host::Xaction *x = hostx;
	Must(x);
	hostx = 0;
	return x;
}

// create the adapter and register with libecap to reach the host application
static const bool Registered =
	libecap::RegisterVersionedService(new Adapter::Service);
