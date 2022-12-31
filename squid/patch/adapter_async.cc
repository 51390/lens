#include "sample.h"
#include "Debug.h"
#include <cstdio>
#include <cstring>
#include <libecap/common/registry.h>
#include <libecap/common/errors.h>
#include <libecap/adapter/service.h>
#include <libecap/adapter/xaction.h>
#include <libecap/host/host.h>
#include <libecap/host/xaction.h>
#include <assert.h>
#include <iostream>
#include <list>
#if HAVE_PTHREAD
#include <pthread.h>
#endif
#include <unistd.h>

/*
 * Warning: This sample code is NOT thread-safe! A production implementation
 * MUST protect and synchronize shared resources. Most, but probably not all
 * relevant places are marked with an "XXX". Other than this important caveat,
 * this sample eCAP adapter illustrates how to orchestrate asynchronous
 * analysis while keeping the thread-unaware host appliaction happy.
 */


namespace Adapter { // not required, but adds clarity

int counter = 0;

class Xaction;
typedef libecap::shared_ptr<Xaction> XactionPointer;

class Service: public libecap::adapter::Service {
	public:
		// About
		virtual std::string uri() const;
		virtual std::string tag() const;
		virtual void describe(std::ostream &os) const;
		virtual bool makesAsyncXactions() const { return true; }

		// Configuration
		virtual void configure(const libecap::Options &cfg);
		virtual void reconfigure(const libecap::Options &cfg);

		// Lifecycle
		virtual void start();
		virtual void suspend(timeval &timeout);
		virtual void resume();
		virtual void stop();
		virtual void retire();

		// Scope
		virtual bool wantsUrl(const char *url) const;

		// Work
		virtual MadeXactionPointer makeXaction(libecap::host::Xaction *hostx);

		// number of transactions still "analysing" their messages
		static int WorkingXactions_; // XXX: not thread-safe!

		static void Resume(const XactionPointer &x);

private:
		// transactions that completed their "analysis"
		typedef std::list<XactionPointer> WaitingXactions;
		static WaitingXactions WaitingXactions_; // XXX: not thread-safe!
};

Adapter::Service::WaitingXactions Adapter::Service::WaitingXactions_;
int Adapter::Service::WorkingXactions_ = 0;

// an async adapter transaction
class Xaction: public libecap::adapter::Xaction {
	public:
		Xaction(libecap::host::Xaction *x);
		virtual ~Xaction();

		// meta-info for the host transaction
		virtual const libecap::Area option(const libecap::Name &name) const;
		virtual void visitEachOption(libecap::NamedValueVisitor &visitor) const;

		// lifecycle
		virtual void start();
		virtual void resume();
		virtual void stop();

		// adapted body transmission control
		virtual void abDiscard() { noBodySupport(); }
		virtual void abMake() { noBodySupport(); }
		virtual void abMakeMore() { noBodySupport(); }
		virtual void abStopMaking() { noBodySupport(); }

		// adapted body content extraction and consumption
		virtual libecap::Area abContent(libecap::size_type, libecap::size_type) { noBodySupport(); return libecap::Area(); }
		virtual void abContentShift(libecap::size_type) { noBodySupport(); }

		// virgin body state notification
		virtual void noteVbContentDone(bool);
		virtual void noteVbContentAvailable();

		// perform (well, simulate) content adaptation
		void analyze();

		// give host control after async analysis
		void tellHostToResume();

		XactionPointer self;

        int id = 0;

	protected:
		void noBodySupport() const;

	private:
		libecap::host::Xaction *hostx; // Host transaction rep
#if HAVE_PTHREAD
		pthread_t thread_; // pthread handler
#endif
};

} // namespace Adapter

std::string Adapter::Service::uri() const {
	return "ecap://e-cap.org/ecap/services/sample/async";
}

std::string Adapter::Service::tag() const {
	return PACKAGE_VERSION;
}

void Adapter::Service::describe(std::ostream &os) const {
	os << "An async adapter from " << PACKAGE_NAME << " v" << PACKAGE_VERSION;
}

void Adapter::Service::configure(const libecap::Options &) {
	if (Debug::Prefix.empty()) {
		Debug::Prefix = "adapter_async: ";
		#if HAVE_PTHREAD
			Debug(flApplication|ilNormal) << "WARNING: This sample eCAP " <<
				"adapter is NOT thread-safe. Sooner or later, it will " <<
				"crash your host application.";
		#else
			Debug(flApplication|ilNormal) << "ERROR: This sample eCAP " <<
				"adapter was built without pthread support. " <<
				"The adapter will not work as intended.";
		#endif /* HAVE_PTHREAD */
	}
	// this service is not really configurable
}

void Adapter::Service::reconfigure(const libecap::Options &) {
	// this service is not configurable
}

void Adapter::Service::start() {
	libecap::adapter::Service::start();
	// custom code would go here, but this service does not have one
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
	return true; // async adapter is applied to all messages
}

void Adapter::Service::suspend(timeval &timeout) {

	// Do not delay waiting transactions more than necessary.
	if (!WaitingXactions_.empty()) {
		timeout.tv_sec = 0;
		timeout.tv_usec = 0;
		return;
	}

	// Do not ignore working transactions for too long:
	// In most cases, the adapter does not know when the async analysis will
	// be over, so using a constant maximum delay such as 300ms is acceptible.
	if (WorkingXactions_) {
		const int maxUsec = 300*1000;
		if (timeout.tv_sec > 0 || timeout.tv_usec > maxUsec) {
			timeout.tv_sec = 0;
			timeout.tv_usec = maxUsec;
		}
		return;
	}

	// otherwise, the host sleep as much as it (or other services) want,
	// preventing "hot idle" state
}

void Adapter::Service::resume() {

	while (!WaitingXactions_.empty()) {
		XactionPointer x = WaitingXactions_.front();
		WaitingXactions_.pop_front();
		x->tellHostToResume();
	}
}

Adapter::Service::MadeXactionPointer
Adapter::Service::makeXaction(libecap::host::Xaction *hostx) {
	Adapter::Xaction *x = new Adapter::Xaction(hostx);
	x->self.reset(x);
	return x->self;
}

void Adapter::Service::Resume(const XactionPointer &x) {
	assert(WorkingXactions_);
	// We are running inside the transaction thread so we cannot call the host
	// application now. We must wait for the host to call our Service::resume.
	// XXX: push_back creates a copy of x, which is not thread-safe
	WaitingXactions_.push_back(x);
}


/* Xaction */

Adapter::Xaction::Xaction(libecap::host::Xaction *x): hostx(x) {
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

extern "C"
void *Analyze(void *arg) {
	static_cast<Adapter::Xaction*>(arg)->analyze();
	return 0;
}

// This method runs inside a non-host thread. Must not call host here.
void Adapter::Xaction::analyze() {
	++Service::WorkingXactions_;
	static int count = 0;
	const int delay = (++count % 4); // 0-3 seconds
	Debug(flApplication|ilNormal) << "adapter_async[" << this << "] analysis ";
	Service::Resume(self);
	self.reset(); // XXX: may not happen if thread is canceled
	--Service::WorkingXactions_; // XXX: may not happen if thread is canceled
}

void Adapter::Xaction::start() {
	Must(hostx);
#if HAVE_PTHREAD
	//Must(pthread_create(&thread_, 0, &Analyze, this) == 0);
#else
#warning No pthread support detected. The adapter_async sample will not work as intended.
	Analyze(this);
#endif
}

void Adapter::Xaction::resume() {
	assert(hostx);
	// make this adapter non-callable
}

void Adapter::Xaction::stop() {
#if HAVE_PTHREAD
	if (hostx)
		pthread_cancel(thread_);
#endif

	hostx = 0;
	// the caller will delete
	// XXX: remove this transaction from the WaitingXactions container!
}

void Adapter::Xaction::noBodySupport() const {
	Must(!"must not be called: async adapter offers no body support");
	// not reached
}

void Adapter::Xaction::noteVbContentDone(bool atEnd)
{
    hostx->vbStopMaking();
}


typedef struct {
    char* payload;
    size_t n;
    int id;
#if HAVE_PTHREAD
    pthread_t thread_; // pthread handler
#endif
} Data;

extern "C"
void* extract(void* arg)
{
    Data* data = (Data*)arg;
    char filename[128];
    memset(filename, 0, sizeof(filename));
    sprintf(filename, "/tmp/response-%d.dump", data->id);

    FILE* f = fopen(filename, "a+");
    fwrite(data->payload, data->n, sizeof(char), f);
    fclose(f);

    //delete data->payload;
    //delete data;

    pthread_exit(0);
    return 0;
}

void Adapter::Xaction::noteVbContentAvailable()
{
    const libecap::Area vb = hostx->vbContent(0, libecap::nsize); // get all vb
    Data* d = new Data();
    d->n = vb.size;
    d->payload = new char(d->n);
    d->id = id;
    memcpy(d->payload, vb.start, d->n * sizeof(char));
    hostx->vbContentShift(vb.size); // we have a copy; do not need vb any more
    
#if HAVE_PTHREAD
	Must(pthread_create(&d->thread_, 0, &extract, d) == 0);
#endif
}

void Adapter::Xaction::tellHostToResume() {
	// if we are stopped during async analysis, stop() tries to cancel the
	// thread, but it is possible that the cancellation comes after the
	// transaction has been added to WaitingXactions.
	if (hostx != 0)
		hostx->resume();
}

// create the adapter and register with libecap to reach the host application
static const bool Registered =
	libecap::RegisterVersionedService(new Adapter::Service);
