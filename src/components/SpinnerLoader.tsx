export default function SpinnerLoader() {
  return (
    <div className="flex flex-col items-center justify-center py-20 gap-6">
      <div className="spinner-wrapper">
        <div className="spinner">
          <div className="spinnerin" />
        </div>
      </div>
      <div className="text-center">
        <p className="text-zinc-300 text-lg">Reunindo informações necessárias</p>
        <p className="text-zinc-500 text-sm mt-1">Em seguida faça seu download :)</p>
      </div>
    </div>
  );
}
